use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::agent::state::{AgentState, AgentView};
use crate::event::AppEvent;

pub fn debug_log(msg: impl std::fmt::Display) {
    let path = socket_dir().join("tui-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default();
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

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
    let dir = socket_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("sock") {
            continue;
        }
        if let Ok(mut stream) = UnixStream::connect(&path).await {
            let _ = stream.write_all(buf.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    }
    Ok(())
}

pub async fn broadcast_notify() -> Result<()> {
    let dir = socket_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("sock") {
            continue;
        }
        if let Ok(mut stream) = UnixStream::connect(&path).await {
            let _ = stream.write_all(b"notify\n").await;
            let _ = stream.shutdown().await;
        }
    }
    Ok(())
}

pub async fn start_listener(path: &Path, tx: mpsc::Sender<AppEvent>) -> Result<()> {
    debug_log(format!("IPC listener starting on {:?}", path));
    let listener = UnixListener::bind(path)?;
    debug_log("IPC listener bound successfully");
    // Semaphore to limit concurrent IPC handlers (prevent resource exhaustion)
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                debug_log(format!("IPC accept error: {}", e));
                continue;
            }
        };
        debug_log(format!("IPC connection accepted (peer: {:?})", peer));
        let tx = tx.clone();
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                debug_log("IPC semaphore full, skipping connection");
                continue;
            }
        };
        tokio::spawn(async move {
            let mut stream = stream;
            let mut buf = Vec::new();
            if let Err(e) = stream.read_to_end(&mut buf).await {
                debug_log(format!("IPC read error: {}", e));
                drop(permit);
                return;
            }
            let data = match String::from_utf8(buf) {
                Ok(s) => s,
                Err(e) => {
                    debug_log(format!("IPC invalid UTF-8: {}", e));
                    drop(permit);
                    return;
                }
            };
            let data = data.trim().to_string();
            if data.is_empty() {
                drop(permit);
                return;
            }
            // Handle each line (supports multiple messages per connection)
            for line in data.lines() {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if line == "notify" {
                    debug_log("IPC received: notify");
                    let ok = tx.try_send(AppEvent::TmuxChanged);
                    debug_log(format!("IPC notify sent: ok={}", ok.is_ok()));
                } else if let Ok(state) = serde_json::from_str::<AgentState>(&line) {
                    debug_log(format!(
                        "IPC received: pane={} status={:?} tools={} event_type={:?}",
                        state.tmux_pane,
                        state.status(),
                        state.active_tools().len(),
                        state.event_label(),
                    ));
                    let ok = tx.try_send(AppEvent::AgentStateUpdate(Box::new(state)));
                    debug_log(format!("IPC state sent to channel: ok={}", ok.is_ok()));
                } else {
                    debug_log(format!(
                        "IPC deserialize FAILED: len={} preview={}",
                        line.len(),
                        &line[..line.len().min(200)]
                    ));
                }
            }
            drop(permit);
        });
    }
}

pub fn cleanup_instance_socket(pid: u32) {
    let path = instance_socket_path(pid);
    if path.exists() {
        std::fs::remove_file(&path).ok();
    }
    let dir = socket_dir();
    if let Ok(mut entries) = std::fs::read_dir(&dir) {
        if entries.next().is_none() {
            std::fs::remove_dir(&dir).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_socket_dir_with_xdg() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        let dir = socket_dir();
        assert_eq!(dir, PathBuf::from("/run/user/1000/chikuwa"));
        std::env::remove_var("XDG_RUNTIME_DIR");
    }

    #[test]
    fn test_socket_dir_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("XDG_RUNTIME_DIR");
        let dir = socket_dir();
        assert_eq!(dir, PathBuf::from("/tmp/chikuwa"));
    }

    #[test]
    fn test_instance_socket_path() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        let path = instance_socket_path(12345);
        assert_eq!(path, PathBuf::from("/run/user/1000/chikuwa/12345.sock"));
        std::env::remove_var("XDG_RUNTIME_DIR");
    }
}
