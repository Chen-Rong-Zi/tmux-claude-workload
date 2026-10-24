# Multi-Instance Broadcast & Fast Shutdown Refactor

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix multiple TUI instances conflicting on a shared socket, and make TUI shutdown fast.

**Architecture:** Replace single fixed Unix socket with a directory of per-instance sockets (`chikuwa/<pid>.sock`). Hook scans the directory and broadcasts state JSON to all running TUI instances. Shutdown collects JoinHandles + uses AtomicBool flag for immediate cancellation of the blocking event loop, runs tmux hook cleanup concurrently.

**Tech Stack:** Rust, tokio, crossterm

---

### Task 1: Core IPC module refactoring

**Files:**
- Modify: `src/ipc.rs`
- Test: `src/ipc.rs` (tests module)

**Changes:**

Replace single-socket API with directory-based multi-socket broadcast API:

- `socket_path()` → Replace with `socket_dir()` returning `$XDG_RUNTIME_DIR/chikuwa/` (fallback `/tmp/chikuwa/`)
- `send_state()` → Replace with `broadcast_state()`: scan `*.sock` in dir, connect to each, send JSON line
- `send_notify()` → Replace with `broadcast_notify()`: same scan-and-send pattern with `"notify\n"`
- `cleanup_socket()` → Replace with `cleanup_instance_socket(pid)`: remove `<pid>.sock`, try to clean up dir if empty
- `start_listener()` → Take explicit socket path as argument instead of computing it internally

- [ ] **Step 1: Write tests for new socket directory functions**

```rust
#[test]
fn test_socket_dir_with_xdg() {
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    let dir = socket_dir();
    assert_eq!(dir, PathBuf::from("/run/user/1000/chikuwa"));
    std::env::remove_var("XDG_RUNTIME_DIR");
}

#[test]
fn test_socket_dir_fallback() {
    std::env::remove_var("XDG_RUNTIME_DIR");
    let dir = socket_dir();
    assert_eq!(dir, PathBuf::from("/tmp/chikuwa"));
}

#[test]
fn test_instance_socket_path() {
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    let path = instance_socket_path(12345);
    assert_eq!(path, PathBuf::from("/run/user/1000/chikuwa/12345.sock"));
    std::env::remove_var("XDG_RUNTIME_DIR");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test ipc::tests -v`
Expected: compile error (functions not defined yet)

- [ ] **Step 3: Implement new socket functions in ipc.rs**

Replace the entire file content:

```rust
use std::path::{Path, PathBuf};

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::agent::state::AgentState;
use crate::event::AppEvent;

pub fn socket_dir() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("chikuwa")
    } else {
        PathBuf::from("/tmp/chikuwa")
    }
}

pub fn instance_socket_path(pid: u32) -> PathBuf {
    socket_dir().join(format!("{}.sock", pid))
}

pub async fn broadcast_state(state: &AgentState) -> Result<()> {
    let json = serde_json::to_string(state)?;
    let mut buf = json;
    buf.push('\n');
    broadcast_to_all(|mut stream| async {
        let _ = stream.write_all(buf.as_bytes()).await;
        let _ = stream.shutdown().await;
    }).await;
    Ok(())
}

pub async fn broadcast_notify() -> Result<()> {
    let buf = b"notify\n";
    broadcast_to_all(|mut stream| async {
        let _ = stream.write_all(buf).await;
        let _ = stream.shutdown().await;
    }).await;
    Ok(())
}

async fn broadcast_to_all<F, Fut>(f: F)
where
    F: Fn(UnixStream) -> Fut + Copy,
    Fut: std::future::Future<Output = ()>,
{
    let dir = socket_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("sock") {
            continue;
        }
        if let Ok(stream) = UnixStream::connect(&path).await {
            f(stream).await;
        }
    }
}

pub async fn start_listener(path: &Path, tx: mpsc::Sender<AppEvent>) -> Result<()> {
    let listener = UnixListener::bind(path)?;
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stream);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if line == "notify" {
                    let _ = tx.send(AppEvent::TmuxChanged).await;
                } else if let Ok(state) = serde_json::from_str::<AgentState>(&line) {
                    let _ = tx.send(AppEvent::AgentStateUpdate(state)).await;
                }
            }
        });
    }
}

pub fn cleanup_instance_socket(pid: u32) {
    let path = instance_socket_path(pid);
    if path.exists() {
        std::fs::remove_file(&path).ok();
    }
    // Try to remove the directory if empty
    let dir = socket_dir();
    if let Ok(mut entries) = std::fs::read_dir(&dir) {
        if entries.next().is_none() {
            std::fs::remove_dir(&dir).ok();
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test ipc::tests -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/ipc.rs
git commit -m "refactor(ipc): replace single socket with directory-based broadcast"
```

### Task 2: Parallel tmux hook unregistration

**Files:**
- Modify: `src/tmux/client.rs`

- [ ] **Step 1: Write test to verify concurrent unregistration**

Actually, unit testing unregister_hooks requires mocking tmux. Instead, just verify the function signature compiles and the logic is correct. The behavior change is that 10 commands now run concurrently instead of sequentially.

Add a compilation test:

```rust
#[test]
fn test_unregister_hooks_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<fn() -> _>(unregister_hooks());
}
```

Hmm, that won't work easily. Let's skip the dedicated test for this and just modify the function.

- [ ] **Step 2: Make unregister_hooks concurrent**

Replace the sequential loop with concurrent spawned tasks:

```rust
pub async fn unregister_hooks() {
    use tokio::process::Command;
    let handles: Vec<_> = HOOK_NAMES.iter().map(|name| {
        let hook_arg = format!("{}[{}]", name, HOOK_INDEX);
        tokio::spawn(async move {
            let _ = Command::new("tmux")
                .arg("set-hook")
                .arg("-gu")
                .arg(&hook_arg)
                .output()
                .await;
        })
    }).collect();
    for h in handles {
        let _ = h.await;
    }
}
```

- [ ] **Step 3: Ensure existing tests still pass**

Run: `cargo test tmux::client::tests -v`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/tmux/client.rs
git commit -m "perf(tmux): run hook unregistration concurrently"
```

### Task 3: Graceful shutdown + socket lifecycle in TUI

**Files:**
- Modify: `src/app.rs`
- Modify: `src/event.rs`

- [ ] **Step 1: Lower event poll interval in event.rs**

Change from 1s to 100ms:

```rust
const TICK_RATE_MS: u64 = 100;
```

In `run_app`, change the tick rate variable.

Actually the tick rate is passed from `run_app`. Let me just change the value:

In `app.rs`:
```rust
// Change:
let tick_rate = Duration::from_secs(1);
// To:
let tick_rate = Duration::from_millis(100);
```

- [ ] **Step 2: Add shutdown flag and JoinHandle collection**

In `run_app`, make these changes:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// Inside run_app, after creating mpsc channel:
let shutdown = Arc::new(AtomicBool::new(false));
let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
```

- [ ] **Step 3: Create and manage instance socket**

```rust
use crate::ipc;

// Before spawning IPC listener, create our socket:
let pid = std::process::id();
let socket_path = ipc::instance_socket_path(pid);
// Ensure directory exists
let socket_dir = ipc::socket_dir();
let _ = std::fs::create_dir_all(&socket_dir);
// Remove stale socket if any
if socket_path.exists() {
    let _ = std::fs::remove_file(&socket_path);
}
```

- [ ] **Step 4: Update all spawn calls to collect handles**

Current code:

Replace each `tokio::spawn(...)` / `tokio::task::spawn_blocking(...)` with:

```rust
// Event loop (blocking) — add shutdown check
let s = shutdown.clone();
let event_tx = tx.clone();
handles.push(tokio::task::spawn_blocking(move || {
    let handle = tokio::runtime::Handle::current();
    handle.block_on(async move {
        loop {
            if s.load(Ordering::Relaxed) {
                break;
            }
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                if let Ok(evt) = event::read() {
                    match evt {
                        Event::Key(key) => {
                            if event_tx.send(AppEvent::Key(key)).await.is_err() {
                                break;
                            }
                        }
                        Event::Mouse(mouse) => {
                            if event_tx.send(AppEvent::Mouse(mouse)).await.is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            } else if event_tx.send(AppEvent::Tick).await.is_err() {
                break;
            }
        }
    });
}));
```

```rust
// IPC listener — pass socket_path
let ipc_tx = tx.clone();
let ipc_path = socket_path.clone();
handles.push(tokio::spawn(async move {
    let _ = ipc::start_listener(&ipc_path, ipc_tx).await;
}));
```

```rust
// Animation ticker
let anim_tx = tx.clone();
handles.push(tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_millis(150));
    loop {
        interval.tick().await;
        if anim_tx.send(AppEvent::AnimationTick).await.is_err() {
            break;
        }
    }
}));
```

```rust
// Usage poller
let usage_tx = tx.clone();
handles.push(tokio::spawn(async move {
    // ... same code as before ...
}));
```

- [ ] **Step 5: Handle Quit action with proper shutdown**

Replace the current quit handling:

```rust
Action::Quit => {
    app.should_quit = true;
}
```

With (after the main loop break before `return Ok(;)`):

```rust
// Inside the match on AppEvent::Key:
Action::Quit => {
    app.should_quit = true;
}
```

Then after the main loop (before `return Ok(())`), add shutdown sequence:

```rust
// === SHUTDOWN ===
// 1. Signal shutdown to all tasks
shutdown.store(true, Ordering::Relaxed);
// 2. Abort all background tasks
for h in &handles {
    h.abort();
}
// 3. Remove the event loop's tx clone by also dropping the original
//    (the receiver rx goes out of scope when run_app returns)
// 4. Wait for tasks with a brief timeout
for h in handles {
    let _ = h.await;
}
// 5. Concurrently unregister tmux hooks
tmux_client::unregister_hooks().await;
// 6. Remove our instance socket
ipc::cleanup_instance_socket(std::process::id());
// 7. Proceed to return and cleanup terminal
```

- [ ] **Step 6: Remove old cleanup code from run()**

In `run()`, remove `tmux_client::unregister_hooks().await;` and `ipc::cleanup_socket();` since those are now handled inside `run_app()`.

- [ ] **Step 7: Run tests**

Run: `cargo test -v`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/event.rs
git commit -m "perf(app): graceful shutdown with join handles and atomic flag"
```

### Task 4: Update hook to broadcast

**Files:**
- Modify: `src/hook.rs`

- [ ] **Step 1: Verify existing hook tests pass**

Run: `cargo test hook::tests -v`
Expected: PASS

- [ ] **Step 2: Change send_state to broadcast_state**

Replace `ipc::send_state(&state).await?;` with `ipc::broadcast_state(&state).await?;`

```rust
// In hook.rs run() function:
// Change line 94:
ipc::send_state(&state).await?;
// To:
ipc::broadcast_state(&state).await?;
```

- [ ] **Step 3: Remove unused imports if any**

Remove `use crate::ipc;` → `use crate::ipc;` is still needed.

- [ ] **Step 4: Run tests**

Run: `cargo test hook::tests -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/hook.rs
git commit -m "refactor(hook): broadcast state to all TUI instances"
```

### Task 5: Update notify command to broadcast

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Change notify to broadcast**

```rust
// In main.rs, Commands::Notify handler:
// Change:
ipc::send_notify().await?;
// To:
ipc::broadcast_notify().await?;
```

- [ ] **Step 2: Commit**

```bash
git add src/main.rs
git commit -m "refactor(main): notify broadcasts to all TUI instances"
```

### Task 6: Remove dead code and verify

**Files:**
- Check: `src/ipc.rs`

- [ ] **Step 1: Verify old send_state and send_notify are not used anywhere**

Run: `cargo build 2>&1 | grep -E "warning|error"`

- [ ] **Step 2: Remove old functions if unused**

Remove `send_state` and `send_notify` functions from `ipc.rs` if they are no longer called anywhere.

- [ ] **Step 3: Full build and test**

Run: `cargo build && cargo test`
Expected: no warnings, all tests pass

- [ ] **Step 4: Format and lint**

```bash
cargo fmt && cargo clippy -- -D warnings
```

- [ ] **Step 5: Final commit**

```bash
git add . && git commit -m "refactor: remove dead IPC code and clean up"
```
