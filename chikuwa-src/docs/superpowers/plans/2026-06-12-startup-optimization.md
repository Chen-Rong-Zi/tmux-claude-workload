# Startup Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce perceived TUI startup time from ~800ms to ~5ms (shell frame), ~25ms (tmux tree), ~120ms (full git info) using incremental rendering.

**Architecture:** Replace the synchronous startup waterfall (`refresh() → merge_git_info() → register_hooks() → draw()`) with three non-blocking stages: (1) draw shell frame immediately, (2) tmux-only refresh inline, (3) background git info fetch with 50ms debounced coalescing. Hook registration is spawned to background.

**Tech Stack:** Rust, tokio, ratatui, crossterm

---

### Task 1: Add standalone `fetch_git_info()` to `git.rs`

**Files:**
- Modify: `src/git.rs`

Extract the uncached git-fetch logic from `GitInfoCache::get()` (lines 129-162) into a standalone public function so background tasks can call it without needing a cache instance.

- [ ] **Step 1: Add `fetch_git_info()` function**

After the `GitInfoCache` impl block (after line 213), add:

```rust
/// Fetch all git info for a path from scratch, without using the cache.
/// Fetches branch, repo name, toplevel, worktree name in parallel,
/// then fetches PR info (depends on branch).
pub async fn fetch_git_info(path: &str) -> Option<GitInfo> {
    let (branch, repo_name, toplevel, worktree_name) = tokio::join!(
        fetch_branch(path),
        fetch_repo_name(path),
        fetch_toplevel(path),
        fetch_worktree_name(path),
    );
    let pr = if let Some(ref b) = branch {
        fetch_pr(path, b).await
    } else {
        None
    };
    Some(GitInfo {
        branch,
        pr,
        repo_name,
        toplevel,
        worktree_name,
    })
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p chikuwa`
Expected: all tests pass (no new tests needed — this is a refactor of existing logic)

- [ ] **Step 3: Commit**

```bash
git add src/git.rs
git commit -m "refactor: extract standalone fetch_git_info function"
```

---

### Task 2: Add `GitInfoReady` and `FlushGitInfo` event variants

**Files:**
- Modify: `src/event.rs`

- [ ] **Step 1: Add new variants to `AppEvent`**

Add `use crate::git::GitInfo;` import, then add two new variants to `AppEvent`:

```rust
/// Git info fetched for a pane path (ready for incremental update).
GitInfoReady { path: String, info: GitInfo },
/// Debounce timer expired — apply all pending git info and redraw.
FlushGitInfo,
```

The enum should look like:

```rust
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    AnimationTick,
    AgentStateUpdate(Box<AgentState>),
    TmuxChanged,
    UsageUpdate(Usage, u64),
    UsageError(String, u64),
    GitInfoReady { path: String, info: GitInfo },
    FlushGitInfo,
}
```

- [ ] **Step 2: Build to verify compilation**

Run: `cargo build 2>&1 | head -20`
Expected: compiles successfully

- [ ] **Step 3: Commit**

```bash
git add src/event.rs
git commit -m "feat: add GitInfoReady and FlushGitInfo event variants"
```

---

### Task 3: Add incremental-update methods to `App`

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add new fields to `App` struct**

After `last_tick_refresh` field (line 213), add:

```rust
/// Pending git info from background fetches, keyed by pane path.
pending_git_info: HashMap<String, crate::git::GitInfo>,
/// True when a FlushGitInfo timer is armed.
git_debounce_active: bool,
```

Initialize both in `App::new()`:

```rust
pending_git_info: HashMap::new(),
git_debounce_active: false,
```

- [ ] **Step 2: Add `refresh_tree_only()` method**

After the existing `refresh()` method (after line 233), add:

```rust
/// Refresh tmux tree without fetching git info.
async fn refresh_tree_only(&mut self) -> Result<()> {
    match tmux_client::fetch_tree(&self.agent_states).await {
        Ok(sessions) => {
            self.sessions = sessions;
            // Skip merge_git_info — git info arrives later via GitInfoReady
            // Skip fixup_nvim_titles — needs toplevel from git info
            self.rebuild_tree();
        }
        Err(_) => {
            self.sessions.clear();
            self.tree_items.clear();
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Add `apply_pending_git_info()` method**

After `refresh_tree_only()`, add:

```rust
/// Apply all pending git info to matching panes and redraw.
fn apply_pending_git_info(&mut self) {
    for (path, info) in self.pending_git_info.drain() {
        for session in &mut self.sessions {
            for window in &mut session.windows {
                for pane in &mut window.panes {
                    if pane.pane_current_path == path {
                        pane.git_info = Some(info.clone());
                    }
                }
            }
        }
    }
    // Re-derive session-level repo_name/toplevel/worktree_name
    for session in &mut self.sessions {
        session.repo_name = session
            .windows
            .iter()
            .flat_map(|w| w.panes.iter())
            .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.repo_name.clone()));
        session.toplevel = session
            .windows
            .iter()
            .flat_map(|w| w.panes.iter())
            .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.toplevel.clone()));
        session.worktree_name = session
            .windows
            .iter()
            .flat_map(|w| w.panes.iter())
            .find_map(|p| p.git_info.as_ref().and_then(|gi| gi.worktree_name.clone()));
    }
    self.fixup_nvim_titles();
    self.rebuild_tree();
}
```

- [ ] **Step 4: Add helper to collect unique pane paths**

Add a standalone function (before `run()`) or a method on `App`. The simplest is a standalone function near `is_subagent_state`:

```rust
/// Collect unique pane paths from sessions.
fn collect_pane_paths(sessions: &[TmuxSession]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for session in sessions {
        for window in &session.windows {
            for pane in &window.panes {
                if seen.insert(pane.pane_current_path.clone()) {
                    paths.push(pane.pane_current_path.clone());
                }
            }
        }
    }
    paths
}
```

- [ ] **Step 5: Build**

Run: `cargo build 2>&1 | head -30`
Expected: compiles successfully

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: add incremental git info methods to App"
```

---

### Task 4: Restructure `run_app()` for incremental rendering

**Files:**
- Modify: `src/app.rs`

This is the core change. The startup flow in `run_app()` changes from:

```
App::new() → refresh() → register_hooks() → spawn tasks → event loop
```

to:

```
App::new() → draw shell frame → refresh_tree_only() → draw tree
  → spawn git background tasks + hook registration → event loop
```

- [ ] **Step 1: Restructure the beginning of `run_app()`**

Replace lines 783-792 (the current `run_app` body start):

**Before (lines 783-792):**
```rust
async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    // Initial data fetch
    app.refresh().await?;

    // Register tmux hooks for instant change notifications (non-fatal on error)
    if let Err(e) = tmux_client::register_hooks().await {
        eprintln!("Warning: failed to register tmux hooks: {}", e);
    }
```

**After:**
```rust
async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    // ── Stage 1: Shell frame (before any I/O) ──
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(3)])
            .split(f.area());
        render_title(f, chunks[0], &app);
        render_status_bar(f, chunks[2], &app.sessions, None, None);
    })?;

    // ── Stage 2: Tmux structure only (no git info) ──
    app.refresh_tree_only().await?;

    // Draw with tmux tree visible
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(3)])
            .split(f.area());
        render_title(f, chunks[0], &app);
        let visible_height = chunks[1].height as usize;
        app.last_width = chunks[1].width;
        app.tree_area = chunks[1];
        // Ensure selected is visible
        let selected_visual = tree::item_to_visual_row(&app.tree_items, app.selected, app.last_width);
        if selected_visual >= app.scroll_offset + visible_height {
            app.scroll_offset = selected_visual.saturating_sub(visible_height - 1);
        }
        if selected_visual < app.scroll_offset {
            app.scroll_offset = selected_visual;
        }
        tree::render(f, chunks[1], &app.tree_items, app.selected, app.scroll_offset, app.anim_frame, &HashMap::new());
        render_status_bar(f, chunks[2], &app.sessions, None, None);
    })?;
```

- [ ] **Step 2: Spawn git background tasks and hook registration**

After the stage-2 draw and before spawning the event/animation/IPC tasks, add:

```rust
    // ── Stage 3: Background git info fetch ──
    let paths = collect_pane_paths(&app.sessions);
    let git_tx = tx.clone();
    for path in paths {
        let tx = git_tx.clone();
        tokio::spawn(async move {
            if let Some(info) = crate::git::fetch_git_info(&path).await {
                let _ = tx.send(AppEvent::GitInfoReady { path, info }).await;
            }
        });
    }

    // Hook registration (background, non-blocking)
    let hook_tx = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = tmux_client::register_hooks().await {
            eprintln!("Warning: failed to register tmux hooks: {}", e);
        }
    });
```

- [ ] **Step 3: Add `GitInfoReady`/`FlushGitInfo` event handling in main loop**

In the event loop's match block (after line 1231, around the `AppEvent::AgentStateUpdate` handler), add:

```rust
                AppEvent::GitInfoReady { path, info } => {
                    app.pending_git_info.insert(path, info);
                    if !app.git_debounce_active {
                        app.git_debounce_active = true;
                        let flush_tx = tx.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            let _ = flush_tx.send(AppEvent::FlushGitInfo).await;
                        });
                    }
                }
                AppEvent::FlushGitInfo => {
                    app.git_debounce_active = false;
                    app.apply_pending_git_info();
                }
```

- [ ] **Step 4: Extract `render_title` and `render_status_bar` helpers**

Since we now draw before the event loop, we need to extract the title bar rendering from the main draw closure into a reusable function, and similarly expose status bar rendering.

Add these as standalone functions (before or after `run_app`):

```rust
/// Render the title bar into the given area.
fn render_title(f: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let has_running = app
        .agent_states
        .values()
        .any(|s| s.status() == crate::agent::state::AgentStatus::Running);
    let title_spans = if has_running {
        // ... wave animation spans (copy from current draw closure) ...
        // ... same animation logic as current draw code for title ...
        // This is identical to the current title rendering code.
        // For brevity: use the exact same animation logic as the existing draw closure.
        let wave_palette = {
            let white: (f32, f32, f32) = (0xff as f32, 0xff as f32, 0xff as f32);
            let purple: (f32, f32, f32) = (0x92 as f32, 0x93 as f32, 0xfe as f32);
            let total = 40;
            let peak = 3;
            let mut palette = Vec::with_capacity(total);
            for i in 0..total {
                let t = if i < peak {
                    i as f32 / peak as f32
                } else if i < peak * 2 {
                    1.0 - (i - peak) as f32 / peak as f32
                } else {
                    0.0
                };
                let t = t * t * (3.0 - 2.0 * t);
                let r = purple.0 + (white.0 - purple.0) * t;
                let g = purple.1 + (white.1 - purple.1) * t;
                let b = purple.2 + (white.2 - purple.2) * t;
                palette.push(Color::Rgb(r as u8, g as u8, b as u8));
            }
            palette
        };
        let plen = wave_palette.len();
        let chikuwa_spans: Vec<Span> = "chikuwa"
            .chars()
            .enumerate()
            .map(|(i, c)| {
                let idx = (plen + i * 2 - app.anim_frame * 3 % plen) % plen;
                Span::styled(
                    c.to_string(),
                    Style::default().fg(wave_palette[idx]).add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        let bolt_style = Style::default().fg(theme::COLOR_YELLOW).add_modifier(Modifier::BOLD);
        let white_style = Style::default().fg(theme::COLOR_WHITE).add_modifier(Modifier::BOLD);
        let mut spans = vec![
            Span::styled("🐧 ", white_style),
            Span::styled(theme::ICON_BOLT, bolt_style),
            Span::styled("  ", white_style),
        ];
        spans.extend(chikuwa_spans);
        spans.push(Span::styled(
            match app.view_mode {
                ViewMode::Tree => "  ",
                ViewMode::Office => ":office  ",
            },
            white_style,
        ));
        spans.push(Span::styled(theme::ICON_BOLT, bolt_style));
        spans.push(Span::styled(" 🐧", white_style));
        spans
    } else {
        let bolt_style = Style::default().fg(theme::COLOR_YELLOW).add_modifier(Modifier::BOLD);
        let white_style = Style::default().fg(theme::COLOR_WHITE).add_modifier(Modifier::BOLD);
        vec![
            Span::styled("🐧 ", white_style),
            Span::styled(theme::ICON_BOLT, bolt_style),
            Span::styled(
                match app.view_mode {
                    ViewMode::Tree => "  chikuwa  ",
                    ViewMode::Office => "  chikuwa:office  ",
                },
                white_style,
            ),
            Span::styled(theme::ICON_BOLT, bolt_style),
            Span::styled(" 🐧", white_style),
        ]
    };
    let title = vec![Line::from(""), Line::from(title_spans)];
    f.render_widget(Paragraph::new(title).alignment(Alignment::Center), area);
}

/// Render the status bar into the given area.
fn render_status_bar(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    sessions: &[TmuxSession],
    usage: Option<std::result::Result<&Usage, &String>>,
    usage_remaining: Option<u64>,
) {
    status_bar::render(f, area, sessions, usage, usage_remaining);
}
```

Then replace the title bar and status bar rendering in the main event loop draw closure with calls to these functions:

```rust
// In the main event loop draw closure, replace:
// ... title rendering code ...
// with:
render_title(f, chunks[0], &app);

// ... status bar rendering code ...
// with:
let usage_remaining = app.usage_next_fetch.map(|t| {
    t.saturating_duration_since(std::time::Instant::now())
        .as_secs()
});
render_status_bar(
    f,
    chunks[2],
    &app.sessions,
    app.usage.as_ref().map(|r| r.as_ref()),
    usage_remaining,
);
```

- [ ] **Step 5: Build and fix any compilation errors**

Run: `cargo build 2>&1 | head -50`
Expected: compiles successfully

If there are errors (e.g., missing imports, type mismatches), fix them.

- [ ] **Step 6: Run tests**

Run: `cargo test -p chikuwa`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "perf: incremental startup with shell frame and background git fetch"
```

---

### Task 5: Run full test suite and cleanup

**Files:**
- Check: `src/app.rs`, `src/event.rs`, `src/git.rs`

- [ ] **Step 1: Format code**

Run: `cargo fmt`

- [ ] **Step 2: Clippy check**

Run: `cargo clippy -- -D warnings`
Expected: no warnings

- [ ] **Step 3: Final test run**

Run: `cargo test -p chikuwa`
Expected: all tests pass

- [ ] **Step 4: Commit formatting/clippy fixes (if any)**

```bash
git add -A
git commit -m "style: format and fix clippy warnings"
```
