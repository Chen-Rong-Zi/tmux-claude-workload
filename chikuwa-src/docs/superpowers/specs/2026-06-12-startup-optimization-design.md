# Startup Optimization: Incremental Rendering

## Problem

chikuwa TUI blocks the first frame until all startup work is done:

```
App::new() → refresh() → merge_git_info() → register_hooks() → draw()
```

Each step is synchronous with the previous. With N git repositories visible in tmux,
startup time scales as O(N) due to serial path iteration in `merge_git_info()`.
Hook registration adds ~300ms from 10 sequential-parallel tmux commands.
The user sees a black screen for 500ms–2s.

## Design

Replace the synchronous waterfall with event-driven incremental rendering.
The screen fills in three stages, each non-blocking and independently rendered.

### Stage 1: Shell frame (~5ms)

Before any I/O, draw a bare title bar and status bar. No tree content.

```
🐧 ⚡  chikuwa  ⚡ 🐧    ← centered title bar
<blank content area>        ← nothing to show yet
Agents: 0  Sessions: 0     ← status bar
```

**Implementation:** In `run_app()`, call `terminal.draw()` immediately after
terminal setup, before `app.refresh()`.

### Stage 2: Tmux structure (~20ms)

Immediately after the shell frame draw (still in `run_app()` before entering
the event loop), run a tmux-only refresh inline:

1. Runs `tmux list-panes -a` (single command, fast)
2. Parses into sessions/windows/panes
3. Builds the tree with `git_info: None` for all panes
4. Calls `rebuild_tree()` and redraws

This avoids waiting for the first Tick event (which has a 100ms poll interval).

**Implementation:** Add a new method `App::refresh_tree_only()` that skips
`merge_git_info()` and `fixup_nvim_titles()`. Call it after the shell frame
draw but before entering the event loop. After it completes, draw a second
frame showing the tmux tree.

### Stage 3: Git info streaming (background)

After the tmux-only refresh, clone the event channel `tx` and spawn parallel
background tasks to fetch git info for each unique pane path. Each task calls
the standalone `fetch_git_info(path)` and sends the result back via
`tx.send(AppEvent::GitInfoReady { path, info })`.

On receiving `GitInfoReady`:
1. Look up all panes matching the path
2. Update `pane.git_info`
3. Update session-level `repo_name`/`toplevel`/`worktree_name`
4. Run `fixup_nvim_titles()` for affected session
5. Call `rebuild_tree()` → redraw

A 50ms debounce coalesces multiple ready events into one redraw to avoid
excessive flicker when several paths finish near-simultaneously.

**Debounce mechanism:** On receiving `GitInfoReady`, do not redraw immediately.
Instead, store the info in a pending map and set a 50ms one-shot timer via
`tokio::spawn(tokio::time::sleep(50ms); tx.send(AppEvent::FlushGitInfo))`.
If another `GitInfoReady` arrives before the timer fires, just update the
pending map. On `FlushGitInfo`, apply all pending info and redraw once.

### Other changes

- **Hook registration**: Spawn to background via `tokio::spawn` instead of
  awaiting. Hooks will be registered ~300ms after first frame, which is fine.
- **Git fetch independence**: Extract a standalone `fetch_git_info(path)` that
  returns `Option<GitInfo>` without involving the cache layer, for use by
  background tasks.

## Files changed

| File | Change |
|------|--------|
| `app.rs` | Restructure `run_app()`: draw first, stage-2 on first event, stage-3 via background tasks |
| `app.rs` | Add `App::refresh_tree_only()` |
| `app.rs` | Add `App::apply_git_info_for_path(path, info)` |
| `app.rs` | Add `AppEvent::GitInfoReady` handler |
| `event.rs` | Add `GitInfoReady` and `FlushGitInfo` variants |
| `git.rs` | Add `pub async fn fetch_git_info(path: &str) -> Option<GitInfo>` |

## Startup timeline

```
Optimized:
  draw()         ─ 5ms    ─→  shell frame visible
  tmux-only      ─ 25ms   ─→  tree visible (no git info)
  git spawn      ─ 25ms   ─→  background fetch starts
  git ready(1)   ─ 80ms   ─→  first repo info appears
  git ready(N)   ─ 120ms  ─→  all info rendered
  hooks ready    ─ 350ms  ─→  tmux change detection active

  User sees TUI at 5ms, full data at ~120ms.
  Previously: all data at 800ms+.
```

## Not changing

- `App::new()` persistence loading (already fast, runs during shell frame)
- Tree rendering logic (no structural changes)
- Hook registration logic (just spawn instead of await)
- Background task structure (follows existing pattern from IPC/usage tasks)
