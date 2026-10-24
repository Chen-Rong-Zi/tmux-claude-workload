use std::collections::HashMap;

use anyhow::{Context, Result};
use futures::future::join_all;
use tokio::process::Command;

use super::types::{TmuxPane, TmuxSession, TmuxWindow};
use crate::agent::claude::ClaudeState;
use crate::agent::codex_state::CodexState;
use crate::agent::detect::detect_agent_source;
use crate::agent::opencode_state::OpenCodeState;
use crate::agent::state::{AgentData, AgentState, AgentStatus};

/// Fetch all tmux sessions/windows/panes and build a tree, merging agent states.
pub async fn fetch_tree(agent_states: &HashMap<String, AgentState>) -> Result<Vec<TmuxSession>> {
    let raw = list_panes_all().await?;
    Ok(build_tree(&raw, agent_states))
}

/// Run `tmux list-panes -a` with a custom format and return raw output.
async fn list_panes_all() -> Result<String> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}\t#{session_attached}\t#{window_index}\t#{window_name}\t#{window_active}\t#{pane_id}\t#{pane_index}\t#{pane_current_command}\t#{pane_active}\t#{pane_current_path}\t#{pane_title}",
        ])
        .output()
        .await
        .context("Failed to execute tmux list-panes")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("tmux list-panes failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse raw tmux output into a tree of sessions > windows > panes.
fn build_tree(raw: &str, agent_states: &HashMap<String, AgentState>) -> Vec<TmuxSession> {
    let mut sessions: Vec<TmuxSession> = Vec::new();
    let mut session_map: HashMap<String, usize> = HashMap::new();

    for line in raw.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 11 {
            continue;
        }

        let session_name = fields[0].to_string();
        let session_attached = fields[1] == "1";
        let window_index: u32 = fields[2].parse().unwrap_or(0);
        let window_name = fields[3].to_string();
        let window_active = fields[4] == "1";
        let pane_id = fields[5].to_string();
        let pane_index: u32 = fields[6].parse().unwrap_or(0);
        let pane_current_command = fields[7].to_string();
        let pane_active = fields[8] == "1";
        let pane_current_path = fields[9].to_string();
        let pane_title = fields[10].to_string();

        let agent_state = agent_states
            .get(&pane_id)
            .and_then(|state| {
                // Verify the pane still hosts an agent process.
                // If the agent is no longer running, don't attach stale state.
                let still_active = match state.source() {
                    crate::agent::state::AgentSource::OpenCode => {
                        // OpenCode tools temporarily change the pane command,
                        // so check pane_title (which persists) instead.
                        crate::agent::detect::is_opencode_pane(
                            Some(&window_name),
                            Some(&pane_title),
                        )
                    }
                    _ => detect_agent_source(
                        &pane_current_command,
                        Some(&window_name),
                        Some(&pane_title),
                    )
                    .is_some(),
                };
                if still_active {
                    Some(state.clone())
                } else {
                    None
                }
            })
            .or_else(|| {
                detected_agent_state(
                    &pane_id,
                    &pane_current_command,
                    Some(&window_name),
                    Some(&pane_title),
                )
            });

        let pane = TmuxPane {
            pane_id,
            pane_index,
            pane_current_command,
            pane_current_path,
            pane_title,
            pane_active,
            agent_state,
            git_info: None,
        };

        let session_idx = if let Some(&idx) = session_map.get(&session_name) {
            idx
        } else {
            let idx = sessions.len();
            sessions.push(TmuxSession {
                session_name: session_name.clone(),
                session_attached,
                windows: Vec::new(),
                repo_name: None,
                toplevel: None,
                worktree_name: None,
            });
            session_map.insert(session_name, idx);
            idx
        };

        let session = &mut sessions[session_idx];

        // Find or create window
        if let Some(window) = session
            .windows
            .iter_mut()
            .find(|w| w.window_index == window_index)
        {
            window.panes.push(pane);
        } else {
            session.windows.push(TmuxWindow {
                window_index,
                window_name,
                window_active,
                panes: vec![pane],
            });
        }
    }

    sessions
}

fn detected_agent_state(
    pane_id: &str,
    command: &str,
    window_name: Option<&str>,
    pane_title: Option<&str>,
) -> Option<AgentState> {
    match detect_agent_source(command, window_name, pane_title)? {
        crate::agent::state::AgentSource::Claude => Some(AgentState::new(
            pane_id.to_string(),
            AgentData::Claude(ClaudeState {
                session_id: None,
                agent_id: None,
                status: AgentStatus::Waiting,
                hook_event_name: "Detected".to_string(),
                event_emoji: "💤".to_string(),
                tool_name: None,
                tool_detail: None,
                active_tools: Vec::new(),
                recent_tools: Vec::new(),
                failure_detail: None,
            }),
        )),
        crate::agent::state::AgentSource::Codex => Some(AgentState::new(
            pane_id.to_string(),
            AgentData::Codex(CodexState::new("Detected", AgentStatus::Waiting, "💤")),
        )),
        crate::agent::state::AgentSource::OpenCode => Some(AgentState::new(
            pane_id.to_string(),
            AgentData::OpenCode(OpenCodeState {
                session_id: None,
                status: AgentStatus::Waiting,
                event_type: Some("Detected".to_string()),
                event_emoji: Some("💤".to_string()),
                tool_name: None,
                tool_detail: None,
                active_tools: Vec::new(),
                recent_tools: Vec::new(),
                is_busy: false,
            }),
        )),
    }
}

/// Switch the current tmux client to a given target (session, window, or pane).
/// When run from within a tmux pane, switch-client automatically targets
/// the client that owns the current pane.
pub async fn switch_to(target: &str) -> Result<()> {
    let output = Command::new("tmux")
        .args(["switch-client", "-t", target])
        .output()
        .await
        .context("Failed to execute tmux switch-client")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("tmux switch-client failed: {}", stderr.trim());
    }

    Ok(())
}

/// Hook names to register for instant tmux change notifications.
const HOOK_NAMES: &[&str] = &[
    "after-select-pane",
    "after-select-window",
    "client-session-changed",
    "session-created",
    "session-closed",
    "session-renamed",
    "window-linked",
    "window-unlinked",
    "window-renamed",
    "pane-exited",
];

/// Hook array index used to avoid conflicts with user hooks.
const HOOK_INDEX: u32 = 42;

/// Register tmux hooks that notify the TUI on structural changes.
/// Registers all hooks in parallel for faster startup.
pub async fn register_hooks() -> Result<()> {
    let exe = std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string();

    // Spawn all hook registrations in parallel
    let handles: Vec<_> = HOOK_NAMES
        .iter()
        .map(|hook_name| {
            let exe = exe.clone();
            let hook_name = *hook_name;
            tokio::spawn(async move {
                let hook_arg = format!("{}[{}]", hook_name, HOOK_INDEX);
                let cmd = format!("run-shell -b '{} notify'", exe);
                let output = Command::new("tmux")
                    .args(["set-hook", "-g", &hook_arg, &cmd])
                    .output()
                    .await
                    .context(format!("Failed to execute tmux set-hook for {}", hook_name))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    anyhow::bail!(
                        "Failed to register tmux hook {}: {}",
                        hook_name,
                        stderr.trim()
                    );
                }
                Ok(())
            })
        })
        .collect();

    // Wait for all registrations and collect errors
    let results = join_all(handles).await;
    for result in results {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => anyhow::bail!("Task join error: {}", e),
        }
    }

    Ok(())
}

/// Unregister tmux hooks concurrently. Ignores errors (hooks may not exist).
pub async fn unregister_hooks() {
    let handles: Vec<_> = HOOK_NAMES
        .iter()
        .map(|name| {
            let hook_arg = format!("{}[{}]", name, HOOK_INDEX);
            tokio::spawn(async move {
                let _ = Command::new("tmux")
                    .arg("set-hook")
                    .arg("-gu")
                    .arg(&hook_arg)
                    .output()
                    .await;
            })
        })
        .collect();
    for h in handles {
        let _ = h.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::claude::ClaudeState;
    use crate::agent::state::{AgentData, AgentState, AgentStatus};

    #[test]
    fn test_build_tree_single_session_single_window() {
        let raw = "main\t1\t0\tzsh\t1\t%0\t0\tbash\t1\t/home/user\tuser@host\n";
        let tree = build_tree(raw, &HashMap::new());

        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].session_name, "main");
        assert!(tree[0].session_attached);
        assert_eq!(tree[0].windows.len(), 1);
        assert_eq!(tree[0].windows[0].window_name, "zsh");
        assert_eq!(tree[0].windows[0].panes.len(), 1);
        assert_eq!(tree[0].windows[0].panes[0].pane_current_command, "bash");
        assert_eq!(tree[0].windows[0].panes[0].pane_current_path, "/home/user");
        assert_eq!(tree[0].windows[0].panes[0].pane_title, "user@host");
    }

    #[test]
    fn test_build_tree_multiple_sessions() {
        let raw = "main\t1\t0\tzsh\t1\t%0\t0\tbash\t1\t/home\t\n\
                    dev\t0\t0\tvim\t1\t%1\t0\tvim\t1\t/tmp\t\n";
        let tree = build_tree(raw, &HashMap::new());

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].session_name, "main");
        assert!(tree[0].session_attached);
        assert_eq!(tree[1].session_name, "dev");
        assert!(!tree[1].session_attached);
    }

    #[test]
    fn test_build_tree_multiple_windows() {
        let raw = "main\t1\t0\tzsh\t1\t%0\t0\tbash\t1\t/home\t\n\
                    main\t1\t1\tvim\t0\t%1\t0\tvim\t1\t/tmp\t\n";
        let tree = build_tree(raw, &HashMap::new());

        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].windows.len(), 2);
        assert_eq!(tree[0].windows[0].window_name, "zsh");
        assert_eq!(tree[0].windows[1].window_name, "vim");
    }

    #[test]
    fn test_build_tree_multiple_panes() {
        let raw = "main\t1\t0\tzsh\t1\t%0\t0\tbash\t1\t/home\t\n\
                    main\t1\t0\tzsh\t1\t%1\t1\tvim\t0\t/tmp\t\n";
        let tree = build_tree(raw, &HashMap::new());

        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].windows.len(), 1);
        assert_eq!(tree[0].windows[0].panes.len(), 2);
        assert_eq!(tree[0].windows[0].panes[0].pane_id, "%0");
        assert_eq!(tree[0].windows[0].panes[1].pane_id, "%1");
    }

    #[test]
    fn test_build_tree_with_agent_state() {
        let raw = "main\t1\t0\tclaude\t1\t%0\t0\tclaude\t1\t/project\t\n";
        let mut agents = HashMap::new();
        agents.insert(
            "%0".to_string(),
            AgentState::new(
                "%0".to_string(),
                AgentData::Claude(ClaudeState {
                    session_id: Some("sess1".to_string()),
                    agent_id: None,
                    status: AgentStatus::Running,
                    hook_event_name: "PreToolUse".to_string(),
                    event_emoji: "🔧".to_string(),
                    tool_name: None,
                    tool_detail: None,
                    active_tools: Vec::new(),
                    recent_tools: Vec::new(),
                    failure_detail: None,
                }),
            ),
        );

        let tree = build_tree(raw, &agents);
        let pane = &tree[0].windows[0].panes[0];
        assert!(pane.agent_state.is_some());
        assert_eq!(
            pane.agent_state.as_ref().unwrap().status(),
            AgentStatus::Running
        );
        assert_eq!(pane.pane_current_path, "/project");
    }

    #[test]
    fn test_build_tree_detects_codex_command_without_hook_state() {
        let raw = "main\t1\t0\tcodex\t1\t%0\t0\tcodex\t1\t/project\tCodex\n";
        let tree = build_tree(raw, &HashMap::new());

        let pane = &tree[0].windows[0].panes[0];
        let state = pane.agent_state.as_ref().expect("codex pane is an agent");
        assert_eq!(state.status(), AgentStatus::Waiting);
        assert_eq!(state.source(), crate::agent::state::AgentSource::Codex);
    }

    #[test]
    fn test_build_tree_detects_codex_binary_command_without_hook_state() {
        let raw = "main\t1\t0\tcodex\t1\t%0\t0\tcodex-aarch64-apple-darwin\t1\t/project\tCodex\n";
        let tree = build_tree(raw, &HashMap::new());

        let pane = &tree[0].windows[0].panes[0];
        let state = pane.agent_state.as_ref().expect("codex pane is an agent");
        assert_eq!(state.status(), AgentStatus::Waiting);
        assert_eq!(state.source(), crate::agent::state::AgentSource::Codex);
    }

    #[test]
    fn test_build_tree_detects_codex_from_title_without_hook_state() {
        let raw = "main\t1\t0\tproject\t1\t%0\t0\tzsh\t1\t/project\tCodex\n";
        let tree = build_tree(raw, &HashMap::new());

        let pane = &tree[0].windows[0].panes[0];
        let state = pane.agent_state.as_ref().expect("codex pane is an agent");
        assert_eq!(state.status(), AgentStatus::Waiting);
        assert_eq!(state.source(), crate::agent::state::AgentSource::Codex);
    }

    #[test]
    fn test_build_tree_detects_claude_command_without_hook_state() {
        let raw = "main\t1\t0\tclaude\t1\t%0\t0\tclaude\t1\t/project\tClaude\n";
        let tree = build_tree(raw, &HashMap::new());

        let pane = &tree[0].windows[0].panes[0];
        let state = pane.agent_state.as_ref().expect("claude pane is an agent");
        assert_eq!(state.status(), AgentStatus::Waiting);
        assert_eq!(state.source(), crate::agent::state::AgentSource::Claude);
    }

    #[test]
    fn test_build_tree_skips_short_lines() {
        let raw = "bad\tline\n\
                    main\t1\t0\tzsh\t1\t%0\t0\tbash\t1\t/home\t\n";
        let tree = build_tree(raw, &HashMap::new());
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn test_stale_opencode_state_cleaned_up() {
        // OpenCode process exited: pane runs bash with plain title → stale state removed
        let raw = "main\t1\t0\tbash\t1\t%0\t0\tbash\t1\t/home\tzsh\n";
        let mut agents = HashMap::new();
        agents.insert(
            "%0".to_string(),
            AgentState::new(
                "%0".to_string(),
                AgentData::OpenCode(OpenCodeState {
                    session_id: Some("ses_stale".to_string()),
                    status: AgentStatus::Running,
                    event_type: Some("tool.completed".to_string()),
                    event_emoji: None,
                    tool_name: None,
                    tool_detail: None,
                    active_tools: Vec::new(),
                    recent_tools: Vec::new(),
                    is_busy: false,
                }),
            ),
        );
        let tree = build_tree(raw, &agents);
        let pane = &tree[0].windows[0].panes[0];
        assert!(
            pane.agent_state.is_none(),
            "stale OpenCode state should be cleaned up when pane no longer shows OC"
        );
    }

    #[test]
    fn test_opencode_state_retained_during_tool() {
        // Tool (bash) is running, but pane title still shows "OC | ..." → state retained
        let raw = "main\t1\t0\tbash\t1\t%0\t0\tbash\t1\t/home\tOC | Greeting\n";
        let mut agents = HashMap::new();
        agents.insert(
            "%0".to_string(),
            AgentState::new(
                "%0".to_string(),
                AgentData::OpenCode(OpenCodeState {
                    session_id: Some("ses_active".to_string()),
                    status: AgentStatus::Running,
                    event_type: Some("tool.running".to_string()),
                    event_emoji: Some("🔧".to_string()),
                    tool_name: Some("bash".to_string()),
                    tool_detail: None,
                    active_tools: Vec::new(),
                    recent_tools: Vec::new(),
                    is_busy: true,
                }),
            ),
        );
        let tree = build_tree(raw, &agents);
        let pane = &tree[0].windows[0].panes[0];
        assert!(
            pane.agent_state.is_some(),
            "OpenCode state should be retained during tool execution"
        );
        assert_eq!(
            pane.agent_state.as_ref().unwrap().status(),
            AgentStatus::Running
        );
    }

    #[test]
    fn test_stale_claude_state_cleaned_up() {
        // Claude exited: pane runs bash → stale Claude state removed
        let raw = "main\t1\t0\tbash\t1\t%0\t0\tbash\t1\t/project\t\n";
        let mut agents = HashMap::new();
        agents.insert(
            "%0".to_string(),
            AgentState::new(
                "%0".to_string(),
                AgentData::Claude(ClaudeState {
                    session_id: Some("ses_stale".to_string()),
                    agent_id: None,
                    status: AgentStatus::Running,
                    hook_event_name: "PreToolUse".to_string(),
                    event_emoji: "🔧".to_string(),
                    tool_name: None,
                    tool_detail: None,
                    active_tools: Vec::new(),
                    recent_tools: Vec::new(),
                    failure_detail: None,
                }),
            ),
        );
        let tree = build_tree(raw, &agents);
        let pane = &tree[0].windows[0].panes[0];
        assert!(
            pane.agent_state.is_none(),
            "stale Claude state should be cleaned up when pane no longer runs claude"
        );
    }

    #[test]
    fn test_claude_state_retained_when_command_matches() {
        // Claude is still running → state retained
        let raw = "main\t1\t0\tclaude\t1\t%0\t0\tclaude\t1\t/project\t\n";
        let mut agents = HashMap::new();
        agents.insert(
            "%0".to_string(),
            AgentState::new(
                "%0".to_string(),
                AgentData::Claude(ClaudeState {
                    session_id: Some("ses_active".to_string()),
                    agent_id: None,
                    status: AgentStatus::Running,
                    hook_event_name: "PreToolUse".to_string(),
                    event_emoji: "🔧".to_string(),
                    tool_name: None,
                    tool_detail: None,
                    active_tools: Vec::new(),
                    recent_tools: Vec::new(),
                    failure_detail: None,
                }),
            ),
        );
        let tree = build_tree(raw, &agents);
        let pane = &tree[0].windows[0].panes[0];
        assert!(pane.agent_state.is_some());
        assert_eq!(
            pane.agent_state.as_ref().unwrap().status(),
            AgentStatus::Running
        );
    }
}
