# Multi-Instance Broadcast & Fast Shutdown Refactor

## Problem

1. **Multiple TUI instances conflict**: Unix socket path is fixed (`chikuwa.sock`), so the last TUI to start "steals" the socket. Earlier instances stop receiving hook events, showing stale agent states.

2. **Slow shutdown**: Three causes accumulate:
   - 10 sequential `tmux set-hook -gu` commands (~100-200ms each = ~1-2s total)
   - Spawned background tasks (IPC listener, animation, usage poller, blocking event loop) receive no shutdown signal
   - Event loop blocks on `event::poll(1s)` — adds up to 1s delay after channel closure

## Design

### 1. IPC: Directory-Based Broadcast

Replace single socket with a directory of per-instance sockets:

```
$XDG_RUNTIME_DIR/chikuwa/
  ├── <pid>.sock   # TUI instance 1
  ├── <pid>.sock   # TUI instance 2
  └── ...
```

**TUI side (listener):**
- On startup: create `$XDG_RUNTIME_DIR/chikuwa/<pid>.sock`, bind, listen
- On exit: delete own socket file

**Hook side (publisher):**
- Scan `$XDG_RUNTIME_DIR/chikuwa/*.sock` for all socket files
- Connect to each, send JSON line
- Skip dead sockets (connection refused) silently — existing behavior

**Notify:**
- Same scan-and-broadcast pattern for notify messages

### 2. Fast Shutdown

**a) Parallel tmux hook unregistration:**
- `unregister_hooks()`: use `futures::future::join_all` to run 10 `set-hook -gu` commands concurrently
- Reduces from ~O(n) to ~O(1) wall-clock time

**b) Graceful task cancellation:**
- Collect `JoinHandle` for all 4 spawned tasks on startup
- On quit: abort all handles
- Also drop the original `tx` sender — tasks detect closed channel as backup

**c) Responsive event loop:**
- Lower poll interval from 1s to 100ms
- On exit, event loop exits within 100ms after tx closure

**d) Socket cleanup:**
- On exit, delete `chikuwa/<pid>.sock` before restoring terminal

### Exit Sequence

```
Quit → should_quit = true → main loop exits
  → abort() all JoinHandles
  → drop(tx) (channel closes)
  → join_all(unregister_hooks) concurrent tmux cleanup
  → delete own socket file
  → restore terminal
  → runtime drops → remaining tasks cancelled
```

## Files Changed

| File | Changes |
|---|---|
| `src/ipc.rs` | Directory-based sockets; `socket_dir()`; hook/notify broadcast |
| `src/hook.rs` | Call new IPC broadcast functions instead of single-socket |
| `src/app.rs` | Collect JoinHandles; abort + drop on quit; lower poll interval |
| `src/tmux/client.rs` | `unregister_hooks()` concurrent via join_all |
| `src/main.rs` | Update notify path |
