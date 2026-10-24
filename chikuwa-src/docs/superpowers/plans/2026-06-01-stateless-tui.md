# Stateless TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the chikuwa TUI stateless so it can be started/stopped freely without losing agent states, git cache, or usage data.

**Architecture:** Hook subprocess writes agent state to a JSONL append-only file on every event. TUI reads this file on startup to reconstruct agent states. Git cache and usage data are persisted as JSON snapshots written by the TUI on shutdown (and periodically). All state files live in `$XDG_RUNTIME_DIR/chikuwa/` alongside the existing socket files.

**Tech Stack:** Rust, tokio, serde_json, std::fs (append/write)

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/persist.rs` | Create | All persistence logic: paths, read/write agent_states JSONL, git cache JSON, usage JSON, compaction |
| `src/hook.rs` | Modify | After IPC broadcast, also append AgentState to `agent_states.jsonl` |
| `src/app.rs` | Modify | On startup: load persisted state into App. On shutdown: save git cache + usage. On AgentStateUpdate: also write to JSONL (so running TUI keeps file up-to-date for future TUI starts) |
| `src/main.rs` | Modify | No changes needed (persist module is used by hook and app) |

---

### Task 1: Create `src/persist.rs` — persistence module with paths and JSONL read/write

**Files:**
- Create: `src/persist.rs`
- Modify: `src/main.rs` (add `mod persist;`)

- [ ] **Step 1: Add `mod persist;` to main.rs**

In `src/main.rs`, add after the existing module declarations:

```rust
mod persist;
```

- [ ] **Step 2: Write `src/persist.rs` with path helpers, agent state JSONL read/write, and tests**

```rust
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::agent::state::{AgentState, AgentStatus};
use crate::git::{GitInfo, PrInfo};
use crate::usage::Usage;

/// Directory for all persistent state files.
pub fn state_dir() -> PathBuf {
    crate::ipc::socket_dir()
}

/// Path to the agent states JSONL file.
pub fn agent_states_path() -> PathBuf {
    state_dir().join("agent_states.jsonl")
}

/// Path to the git cache JSON file.
pub fn git_cache_path() -> PathBuf {
    state_dir().join("git_cache.json")
}

/// Path to the usage JSON file.
pub fn usage_path() -> PathBuf {
    state_dir().join("usage.json")
}

/// Append an AgentState as a single JSON line to the JSONL file.
/// Creates the file and parent directory if they don't exist.
pub fn append_agent_state(state: &AgentState) -> anyhow::Result<()> {
    let path = agent_states_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let mut json = serde_json::to_string(state)?;
    json.push('\n');
    file.write_all(json.as_bytes())?;
    Ok(())
}

/// Reconstruct the current agent states from a JSONL event log.
/// Reads all lines, applies them in order using the same merge logic as the TUI:
/// - `Ended` → remove
/// - Otherwise → insert (last write wins for the same tmux_pane)
///
/// Also filters out states that are likely stale (older than 24 hours),
/// since the TUI that wrote them may have been gone for a long time.
pub fn load_agent_states_from(path: &std::path::Path) -> HashMap<String, AgentState> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let stale_threshold = 24 * 60 * 60; // 24 hours

    let mut states: HashMap<String, AgentState> = HashMap::new();
    let reader = std::io::BufReader::new(file);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let state: AgentState = match serde_json::from_str(line) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Skip stale entries (older than 24 hours)
        if now.saturating_sub(state.updated_at) > stale_threshold {
            continue;
        }

        if state.state == AgentStatus::Ended {
            states.remove(&state.tmux_pane);
        } else {
            // Last write wins — but preserve session_id if incoming is None
            if let Some(existing) = states.get(&state.tmux_pane) {
                let mut merged = state;
                if merged.session_id.is_none() {
                    merged.session_id = existing.session_id.clone();
                }
                states.insert(merged.tmux_pane.clone(), merged);
            } else {
                states.insert(state.tmux_pane.clone(), state);
            }
        }
    }

    states
}

/// Load agent states from the default path.
pub fn load_agent_states() -> HashMap<String, AgentState> {
    load_agent_states_from(&agent_states_path())
}

/// Compact the agent states JSONL file by rewriting it with only the current
/// live states. Call this periodically (e.g., on TUI startup) to prevent
/// unbounded file growth.
pub fn compact_agent_states_to(
    states: &HashMap<String, AgentState>,
    path: &Path,
) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    for state in states.values() {
        let mut json = serde_json::to_string(state)?;
        json.push('\n');
        file.write_all(json.as_bytes())?;
    }
    Ok(())
}

/// Compact agent states at the default path.
pub fn compact_agent_states(states: &HashMap<String, AgentState>) -> anyhow::Result<()> {
    compact_agent_states_to(states, &agent_states_path())
}

// ---- Git cache persistence ----

/// Serializable git cache entry.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GitCacheEntry {
    pub path: String,
    pub branch: Option<String>,
    pub pr: Option<PrInfo>,
    pub repo_name: Option<String>,
    pub toplevel: Option<String>,
    pub worktree_name: Option<String>,
}

/// Serializable wrapper for the entire git cache.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GitCacheSnapshot {
    pub entries: Vec<GitCacheEntry>,
}

/// Save git cache snapshot to disk.
pub fn save_git_cache(entries: &[GitCacheEntry]) -> anyhow::Result<()> {
    let path = git_cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let snapshot = GitCacheSnapshot {
        entries: entries.to_vec(),
    };
    let json = serde_json::to_string_pretty(&snapshot)?;
    // Atomic write: write to temp file then rename
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Load git cache snapshot from disk.
pub fn load_git_cache() -> Option<Vec<GitCacheEntry>> {
    let path = git_cache_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let snapshot: GitCacheSnapshot = serde_json::from_str(&data).ok()?;
    Some(snapshot.entries)
}

// ---- Usage persistence ----

/// Serializable usage data with timestamp.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct UsageSnapshot {
    pub five_hour: f64,
    pub seven_day: f64,
    /// When this snapshot was saved (unix timestamp).
    pub saved_at: u64,
}

/// Save usage data to disk.
pub fn save_usage(usage: &Usage) -> anyhow::Result<()> {
    let path = usage_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let snapshot = UsageSnapshot {
        five_hour: usage.five_hour,
        seven_day: usage.seven_day,
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let json = serde_json::to_string_pretty(&snapshot)?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Load usage data from disk.
pub fn load_usage() -> Option<Usage> {
    let path = usage_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let snapshot: UsageSnapshot = serde_json::from_str(&data).ok()?;
    Some(Usage {
        five_hour: snapshot.five_hour,
        seven_day: snapshot.seven_day,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::path::Path;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("chikuwa_persist_test");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn write_jsonl(path: &Path, lines: &[&str]) {
        let mut f = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
    }

    #[test]
    fn test_append_and_load_agent_states() {
        let dir = temp_dir();
        let path = dir.join("test_append.jsonl");
        let _ = std::fs::remove_file(&path);

        let state1 = AgentState::new("%0".to_string(), AgentStatus::Running);
        let state2 = AgentState::new("%1".to_string(), AgentStatus::Waiting);

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&state1).unwrap(),
                &serde_json::to_string(&state2).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        assert_eq!(states.len(), 2);
        assert!(states.contains_key("%0"));
        assert!(states.contains_key("%1"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_agent_states_ended_removes() {
        let dir = temp_dir();
        let path = dir.join("test_ended.jsonl");
        let _ = std::fs::remove_file(&path);

        let running = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            state: AgentStatus::Running,
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            hook_event_name: None,
            tool_name: None,
            tool_detail: None,
            tools: Vec::new(),
        };
        let mut ended = running.clone();
        ended.state = AgentStatus::Ended;

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&running).unwrap(),
                &serde_json::to_string(&ended).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        assert!(!states.contains_key("%0"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_agent_states_last_write_wins() {
        let dir = temp_dir();
        let path = dir.join("test_merge.jsonl");
        let _ = std::fs::remove_file(&path);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let state1 = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: Some("old-session".to_string()),
            state: AgentStatus::Running,
            updated_at: now - 10,
            hook_event_name: None,
            tool_name: None,
            tool_detail: None,
            tools: Vec::new(),
        };
        let mut state2 = state1.clone();
        state2.state = AgentStatus::Waiting;
        state2.session_id = None; // incoming has no session_id
        state2.updated_at = now;

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&state1).unwrap(),
                &serde_json::to_string(&state2).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        let result = states.get("%0").unwrap();
        assert_eq!(result.state, AgentStatus::Waiting);
        // session_id preserved from earlier entry
        assert_eq!(result.session_id, Some("old-session".to_string()));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_agent_states_stale_filtered() {
        let dir = temp_dir();
        let path = dir.join("test_stale.jsonl");
        let _ = std::fs::remove_file(&path);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let fresh = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            state: AgentStatus::Running,
            updated_at: now,
            hook_event_name: None,
            tool_name: None,
            tool_detail: None,
            tools: Vec::new(),
        };
        let mut stale = fresh.clone();
        stale.tmux_pane = "%1".to_string();
        stale.updated_at = now - 25 * 60 * 60; // 25 hours ago

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&fresh).unwrap(),
                &serde_json::to_string(&stale).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        assert!(states.contains_key("%0"));
        assert!(!states.contains_key("%1")); // stale filtered out

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_git_cache_roundtrip() {
        let entries = vec![GitCacheEntry {
            path: "/home/user/project".to_string(),
            branch: Some("main".to_string()),
            pr: Some(PrInfo {
                number: 42,
                title: "Fix bug".to_string(),
            }),
            repo_name: Some("owner/repo".to_string()),
            toplevel: Some("/home/user/project".to_string()),
            worktree_name: None,
        }];
        let json = serde_json::to_string_pretty(&GitCacheSnapshot {
            entries: entries.clone(),
        })
        .unwrap();
        let parsed: GitCacheSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, "/home/user/project");
        assert_eq!(parsed.entries[0].branch, Some("main".to_string()));
    }

    #[test]
    fn test_usage_roundtrip() {
        let usage = Usage {
            five_hour: 0.63,
            seven_day: 0.19,
        };
        let json = serde_json::to_string_pretty(&UsageSnapshot {
            five_hour: usage.five_hour,
            seven_day: usage.seven_day,
            saved_at: 1234567890,
        })
        .unwrap();
        let parsed: UsageSnapshot = serde_json::from_str(&json).unwrap();
        assert!((parsed.five_hour - 0.63).abs() < f64::EPSILON);
        assert!((parsed.seven_day - 0.19).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compact_agent_states() {
        let dir = temp_dir();
        let path = dir.join("test_compact.jsonl");
        let _ = std::fs::remove_file(&path);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut states = HashMap::new();
        states.insert(
            "%0".to_string(),
            AgentState {
                tmux_pane: "%0".to_string(),
                session_id: Some("s1".to_string()),
                state: AgentStatus::Running,
                updated_at: now,
                hook_event_name: None,
                tool_name: None,
                tool_detail: None,
                tools: Vec::new(),
            },
        );

        compact_agent_states_to(&states, &path).unwrap();

        // Read back and verify
        let data = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = data.trim().lines().collect();
        assert_eq!(lines.len(), 1);
        let parsed: AgentState = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.tmux_pane, "%0");

        let _ = std::fs::remove_file(&path);
    }
}
```

- [ ] **Step 3: Add `Serialize` derive to `PrInfo` in `src/git.rs`**

The `PrInfo` struct needs `Serialize` for persistence. Change the derive:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrInfo {
    pub number: u32,
    pub title: String,
}
```

Also add the import at the top of `src/git.rs`:

```rust
use serde::{Deserialize, Serialize};
```

And remove the old `use serde::Deserialize;` line (it's replaced by the combined import above).

- [ ] **Step 4: Build and verify tests pass**

Run: `cargo test`
Expected: All tests pass, including the new `persist` module tests.

- [ ] **Step 5: Commit**

```bash
git add src/persist.rs src/main.rs src/git.rs
git commit -m "feat: add persistence module for stateless TUI"
```

---

### Task 2: Hook writes agent state to JSONL on every event

**Files:**
- Modify: `src/hook.rs`

- [ ] **Step 1: Add `append_agent_state` call in `hook.rs`**

In `src/hook.rs`, after the `ipc::broadcast_state(&state).await?;` line (line 94), add the persistence write. This must be non-blocking — we write to file synchronously (it's fast) but don't fail the hook if the write fails.

Change the `run()` function's final section from:

```rust
    ipc::broadcast_state(&state).await?;

    Ok(())
```

to:

```rust
    ipc::broadcast_state(&state).await?;

    // Persist to JSONL so TUI can restore state on restart
    if let Err(e) = crate::persist::append_agent_state(&state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }

    Ok(())
```

- [ ] **Step 2: Build and verify tests pass**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/hook.rs
git commit -m "feat: hook writes agent state to JSONL for persistence"
```

---

### Task 3: TUI loads persisted agent states on startup

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add `persist` import in `src/app.rs`**

Add to the imports at the top:

```rust
use crate::persist;
```

- [ ] **Step 2: Load persisted agent states in `App::new()`**

Replace the `App::new()` method:

```rust
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            tree_items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            collapsed: HashSet::new(),
            should_quit: false,
            agent_states: persist::load_agent_states(),
            git_cache: GitInfoCache::new(),
            anim_frame: 0,
            nvim_title_cache: HashMap::new(),
            last_width: 80,
            tree_area: ratatui::layout::Rect::default(),
            user_navigated: false,
            usage: None,
            usage_next_fetch: None,
        }
    }
```

- [ ] **Step 3: Compact JSONL after loading**

After loading, compact the JSONL file so it doesn't grow unboundedly. Add this right after `App::new()` in `run_app`, before the `app.refresh()` call:

In `run_app()`, change:

```rust
    let mut app = App::new();

    // Open event log file if --store-events is enabled
```

to:

```rust
    let mut app = App::new();

    // Compact the JSONL log on startup (remove stale/redundant entries)
    if let Err(e) = persist::compact_agent_states(&app.agent_states) {
        eprintln!("Warning: failed to compact agent states: {}", e);
    }

    // Open event log file if --store-events is enabled
```

- [ ] **Step 4: Also persist agent state on each update in the event loop**

In the `AppEvent::AgentStateUpdate(state)` handler, after the merge logic and `app.merge_agent_states()`, add persistence. Find the line:

```rust
                    app.merge_agent_states();
```

and add after it:

```rust
                    // Persist updated state to JSONL
                    if let Err(e) = persist::append_agent_state(&state) {
                        eprintln!("Warning: failed to persist agent state: {}", e);
                    }
```

This ensures the JSONL file stays current even when the TUI is running, so a future TUI start will see the latest state.

- [ ] **Step 5: Build and verify tests pass**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: TUI loads persisted agent states on startup"
```

---

### Task 4: TUI saves and loads git cache on shutdown/startup

**Files:**
- Modify: `src/git.rs`
- Modify: `src/app.rs`

- [ ] **Step 1: Add `to_cache_entries` and `populate_from_entries` methods to `GitInfoCache`**

In `src/git.rs`, add these two methods to the `impl GitInfoCache` block, after the `retain_paths` method:

```rust
    /// Export cache entries for persistence.
    pub fn to_cache_entries(&self) -> Vec<crate::persist::GitCacheEntry> {
        self.entries
            .iter()
            .map(|(path, entry)| crate::persist::GitCacheEntry {
                path: path.to_string_lossy().to_string(),
                branch: entry.git_info.branch.clone(),
                pr: entry.git_info.pr.clone(),
                repo_name: entry.git_info.repo_name.clone(),
                toplevel: entry.git_info.toplevel.clone(),
                worktree_name: entry.git_info.worktree_name.clone(),
            })
            .collect()
    }

    /// Populate cache from persisted entries. Skips entries whose paths
    /// no longer exist on disk.
    pub fn populate_from_entries(&mut self, entries: Vec<crate::persist::GitCacheEntry>) {
        let now = Instant::now();
        for entry in entries {
            if !std::path::Path::new(&entry.path).exists() {
                continue;
            }
            let path_buf = PathBuf::from(&entry.path);
            self.entries.insert(
                path_buf,
                CacheEntry {
                    git_info: GitInfo {
                        branch: entry.branch,
                        pr: entry.pr,
                        repo_name: entry.repo_name,
                        toplevel: entry.toplevel,
                        worktree_name: entry.worktree_name,
                    },
                    branch_fetched_at: now,
                    pr_fetched_at: now,
                    repo_name_fetched: true,
                    toplevel_fetched: true,
                    worktree_fetched: true,
                },
            );
        }
    }
```

- [ ] **Step 2: Load git cache on startup in `App::new()`**

Change `App::new()` to populate the git cache from persisted data:

```rust
    pub fn new() -> Self {
        let mut git_cache = GitInfoCache::new();
        if let Some(entries) = persist::load_git_cache() {
            git_cache.populate_from_entries(entries);
        }

        Self {
            sessions: Vec::new(),
            tree_items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            collapsed: HashSet::new(),
            should_quit: false,
            agent_states: persist::load_agent_states(),
            git_cache,
            anim_frame: 0,
            nvim_title_cache: HashMap::new(),
            last_width: 80,
            tree_area: ratatui::layout::Rect::default(),
            user_navigated: false,
            usage: None,
            usage_next_fetch: None,
        }
    }
```

- [ ] **Step 3: Save git cache on shutdown**

In `run_app()`, in the shutdown section at the bottom, before the `Ok(())`, add:

Change:

```rust
    tmux_client::unregister_hooks().await;
    ipc::cleanup_instance_socket(std::process::id());

    Ok(())
```

to:

```rust
    tmux_client::unregister_hooks().await;
    ipc::cleanup_instance_socket(std::process::id());

    // Persist git cache on shutdown
    let git_entries = app.git_cache.to_cache_entries();
    if let Err(e) = persist::save_git_cache(&git_entries) {
        eprintln!("Warning: failed to save git cache: {}", e);
    }

    Ok(())
```

- [ ] **Step 4: Build and verify tests pass**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git.rs src/app.rs
git commit -m "feat: persist git cache across TUI restarts"
```

---

### Task 5: TUI saves and loads usage data on shutdown/startup

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Load usage data on startup in `App::new()`**

Change `App::new()` to load persisted usage:

```rust
    pub fn new() -> Self {
        let mut git_cache = GitInfoCache::new();
        if let Some(entries) = persist::load_git_cache() {
            git_cache.populate_from_entries(entries);
        }

        Self {
            sessions: Vec::new(),
            tree_items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            collapsed: HashSet::new(),
            should_quit: false,
            agent_states: persist::load_agent_states(),
            git_cache,
            anim_frame: 0,
            nvim_title_cache: HashMap::new(),
            last_width: 80,
            tree_area: ratatui::layout::Rect::default(),
            user_navigated: false,
            usage: persist::load_usage().map(Ok),
            usage_next_fetch: None,
        }
    }
```

- [ ] **Step 2: Save usage data on shutdown**

In the shutdown section of `run_app()`, after the git cache save, add usage persistence:

```rust
    // Persist git cache on shutdown
    let git_entries = app.git_cache.to_cache_entries();
    if let Err(e) = persist::save_git_cache(&git_entries) {
        eprintln!("Warning: failed to save git cache: {}", e);
    }

    // Persist usage data on shutdown
    if let Some(Ok(ref usage)) = app.usage {
        if let Err(e) = persist::save_usage(usage) {
            eprintln!("Warning: failed to save usage data: {}", e);
        }
    }
```

- [ ] **Step 3: Build and verify tests pass**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: persist usage data across TUI restarts"
```

---

### Task 6: Remove the old `--store-events` flag (replaced by persistent JSONL)

**Files:**
- Modify: `src/main.rs`
- Modify: `src/app.rs`

The `--store-events` flag and the `event_log` mechanism are now superseded by the always-on JSONL persistence. We remove them to avoid confusion and code duplication.

- [ ] **Step 1: Remove `store_events` from CLI in `src/main.rs`**

Remove the `store_events` field from the `Cli` struct and pass nothing to `app::run`:

Change:

```rust
#[derive(Parser)]
#[command(name = "chikuwa", about = "tmux AI Agent monitor TUI", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Store all received hook events to a JSONL file for debugging
    #[arg(long)]
    store_events: bool,
}
```

to:

```rust
#[derive(Parser)]
#[command(name = "chikuwa", about = "tmux AI Agent monitor TUI", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}
```

Change the `app::run` call:

```rust
            app::run(cli.store_events).await?;
```

to:

```rust
            app::run().await?;
```

- [ ] **Step 2: Remove `store_events` parameter and `event_log` from `src/app.rs`**

In `src/app.rs`:

1. Change `pub async fn run(store_events: bool)` to `pub async fn run()`

2. Change `let result = run_app(&mut terminal, store_events).await;` to `let result = run_app(&mut terminal).await;`

3. Remove the `store_events` parameter from `run_app`:

Change:
```rust
async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    store_events: bool,
) -> Result<()> {
```
to:
```rust
async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
```

4. Remove the `event_log` variable and the `event_log_path` function. Delete the entire block:

```rust
    // Open event log file if --store-events is enabled
    let mut event_log: Option<std::fs::File> = if store_events {
        let path = event_log_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .context(format!("Failed to open event log: {}", path.display()))?,
        )
    } else {
        None
    };
```

5. Remove the `event_log_path` function:

```rust
/// Returns the event log file path.
fn event_log_path() -> std::path::PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        std::path::PathBuf::from(runtime_dir)
            .join("chikuwa")
            .join("events.jsonl")
    } else {
        std::path::PathBuf::from("/tmp/chikuwa/events.jsonl")
    }
}
```

6. Remove the event log writing in `AppEvent::AgentStateUpdate` handler. Delete these lines:

```rust
                    if let Some(ref mut log) = event_log {
                        if let Ok(json) = serde_json::to_string(&state) {
                            let _ = writeln!(log, "{}", json);
                        }
                    }
```

- [ ] **Step 3: Build and verify tests pass**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs src/app.rs
git commit -m "refactor: remove --store-events flag, replaced by always-on JSONL persistence"
```

---

### Task 7: Final verification — full build, clippy, and manual test

- [ ] **Step 1: Run full verification**

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

Expected: All pass with no warnings.

- [ ] **Step 2: Manual smoke test**

1. Start the TUI with `cargo run`
2. Verify it starts quickly and shows agent states (if any were previously persisted)
3. Quit the TUI with `q`
4. Restart with `cargo run`
5. Verify agent states are still visible from the previous session
6. Check that `$XDG_RUNTIME_DIR/chikuwa/agent_states.jsonl` exists and contains valid JSON lines
7. Check that `$XDG_RUNTIME_DIR/chikuwa/git_cache.json` exists after a clean shutdown
8. Check that `$XDG_RUNTIME_DIR/chikuwa/usage.json` exists after a clean shutdown

- [ ] **Step 3: Final commit if any cleanup needed**

```bash
git add -A
git commit -m "chore: final cleanup for stateless TUI persistence"
```
