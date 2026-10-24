# Hook Parser Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the hook parsing system into a trait-based extensible architecture and adapt all 28 Claude Code hook events with emoji + status mapping.

**Architecture:** Create a `HookParser` trait in `agent/parser.rs` that defines the interface for parsing raw JSON input into `AgentState`. Implement `ClaudeHookParser` for Claude Code (all 28 events) and `OpenCodeHookParser` for OpenCode. Each event maps to a `(AgentStatus, emoji, display_detail)` tuple. The `AgentState` struct gains an `event_emoji` field. The UI renders the emoji alongside status text.

**Tech Stack:** Rust, serde_json, ratatui

---

### Complete Claude Code Event Mapping

| Event | Emoji | AgentStatus | Detail Source |
|-------|-------|-------------|---------------|
| SessionStart | 🚀 | Started | — |
| Setup | ⚙️ | Started | — |
| InstructionsLoaded | 📄 | Running | `instructions_file` |
| UserPromptSubmit | 💭 | Running | — |
| UserPromptExpansion | ⚡ | Running | `expanded_text` |
| MessageDisplay | 💬 | Running | — |
| PreToolUse | 🔧 | Running | `{tool}: {detail}` |
| PermissionRequest | 🔐 | Permission | `tool_name` |
| PostToolUse | ✅ | Running | (update tools) |
| PostToolUseFailure | ❌ | Running | (update tools) |
| PostToolBatch | 📦 | Running | (batch complete) |
| PermissionDenied | 🚫 | Running | `tool_name` |
| Notification | 🔔 | Running | `message` or Permission if permission_prompt |
| Stop | 💤 | Waiting | — |
| StopFailure | ⚠️ | Waiting | — |
| SubagentStart | 🤖 | Running | (create subagent) |
| SubagentStop | 🏁 | Ended | (remove subagent) |
| TaskCreated | 📋 | Running | `task_description` |
| TaskCompleted | ✔️ | Running | `task_description` |
| TeammateIdle | 👥 | Waiting | — |
| ConfigChange | ⚙️ | Running | — |
| CwdChanged | 📁 | Running | `cwd` |
| FileChanged | 📝 | Running | `file_path` |
| WorktreeCreate | 🌳 | Running | `worktree_path` |
| WorktreeRemove | 🗑️ | Running | `worktree_path` |
| PreCompact | 🗜️ | Running | — |
| PostCompact | 📦 | Running | — |
| SessionEnd | 🏁 | Ended | — |
| Elicitation | ❓ | Permission | `tool_name` |
| ElicitationResult | ✅ | Running | — |

---

### Task 1: Add `event_emoji` field to `AgentState`

**Files:**
- Modify: `src/agent/state.rs:33-49` (AgentState struct)
- Modify: `src/agent/state.rs:52-64` (AgentState::new)
- Modify: `src/agent/state.rs:126-145` (test_agent_state_roundtrip_json)

- [ ] **Step 1: Add `event_emoji` field to `AgentState` struct**

In `src/agent/state.rs`, add the field after `hook_event_name`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub tmux_pane: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub state: AgentStatus,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_event_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolInfo>,
}
```

- [ ] **Step 2: Add `event_emoji: None` to `AgentState::new`**

```rust
pub fn new(tmux_pane: String, state: AgentStatus) -> Self {
    Self {
        tmux_pane,
        session_id: None,
        agent_id: None,
        state,
        updated_at: now(),
        hook_event_name: None,
        event_emoji: None,
        tool_name: None,
        tool_detail: None,
        tools: Vec::new(),
    }
}
```

- [ ] **Step 3: Update `test_agent_state_roundtrip_json` to include `event_emoji`**

In `src/agent/state.rs`, update the test:

```rust
#[test]
fn test_agent_state_roundtrip_json() {
    let state = AgentState {
        tmux_pane: "%5".to_string(),
        session_id: Some("abc123".to_string()),
        agent_id: None,
        state: AgentStatus::Running,
        updated_at: 1234567890,
        hook_event_name: None,
        event_emoji: None,
        tool_name: None,
        tool_detail: None,
        tools: Vec::new(),
    };
    // ... rest unchanged
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/agent/state.rs
git commit -m "feat: add event_emoji field to AgentState"
```

---

### Task 2: Create `HookParser` trait and `ClaudeHookParser`

**Files:**
- Create: `src/agent/parser.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Create `src/agent/parser.rs` with the trait and ClaudeHookParser**

```rust
use anyhow::{Context, Result};
use serde::Deserialize;

use super::state::{AgentState, AgentStatus, ToolInfo};

/// Result of parsing a hook event.
pub struct ParseResult {
    pub state: AgentState,
    /// Whether this event should be suppressed (not sent to TUI).
    pub suppress: bool,
}

/// Trait for parsing raw hook input into AgentState.
pub trait HookParser {
    /// Parse a raw JSON string from stdin into a ParseResult.
    fn parse(&self, pane_id: String, raw_json: &str) -> Result<ParseResult>;
}

// ─── Claude Code Hook Parser ───────────────────────────────────────────

/// Input JSON from Claude Code hooks (stdin).
#[derive(Debug, Deserialize)]
struct ClaudeHookInput {
    hook_event_name: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    instructions_file: Option<String>,
    #[serde(default)]
    expanded_text: Option<String>,
}

/// Event mapping entry: (status, emoji, detail_extractor)
struct EventMapping {
    status: AgentStatus,
    emoji: &'static str,
}

/// Get the event mapping for a Claude Code hook event name.
fn claude_event_mapping(event: &str) -> Option<EventMapping> {
    match event {
        "SessionStart" => Some(EventMapping { status: AgentStatus::Started, emoji: "🚀" }),
        "Setup" => Some(EventMapping { status: AgentStatus::Started, emoji: "⚙️" }),
        "InstructionsLoaded" => Some(EventMapping { status: AgentStatus::Running, emoji: "📄" }),
        "UserPromptSubmit" => Some(EventMapping { status: AgentStatus::Running, emoji: "💭" }),
        "UserPromptExpansion" => Some(EventMapping { status: AgentStatus::Running, emoji: "⚡" }),
        "MessageDisplay" => Some(EventMapping { status: AgentStatus::Running, emoji: "💬" }),
        "PreToolUse" => Some(EventMapping { status: AgentStatus::Running, emoji: "🔧" }),
        "PermissionRequest" => Some(EventMapping { status: AgentStatus::Permission, emoji: "🔐" }),
        "PostToolUse" => Some(EventMapping { status: AgentStatus::Running, emoji: "✅" }),
        "PostToolUseFailure" => Some(EventMapping { status: AgentStatus::Running, emoji: "❌" }),
        "PostToolBatch" => Some(EventMapping { status: AgentStatus::Running, emoji: "📦" }),
        "PermissionDenied" => Some(EventMapping { status: AgentStatus::Running, emoji: "🚫" }),
        "Notification" => None, // handled specially
        "Stop" => Some(EventMapping { status: AgentStatus::Waiting, emoji: "💤" }),
        "StopFailure" => Some(EventMapping { status: AgentStatus::Waiting, emoji: "⚠️" }),
        "SubagentStart" => Some(EventMapping { status: AgentStatus::Running, emoji: "🤖" }),
        "SubagentStop" => Some(EventMapping { status: AgentStatus::Ended, emoji: "🏁" }),
        "TaskCreated" => Some(EventMapping { status: AgentStatus::Running, emoji: "📋" }),
        "TaskCompleted" => Some(EventMapping { status: AgentStatus::Running, emoji: "✔️" }),
        "TeammateIdle" => Some(EventMapping { status: AgentStatus::Waiting, emoji: "👥" }),
        "ConfigChange" => Some(EventMapping { status: AgentStatus::Running, emoji: "⚙️" }),
        "CwdChanged" => Some(EventMapping { status: AgentStatus::Running, emoji: "📁" }),
        "FileChanged" => Some(EventMapping { status: AgentStatus::Running, emoji: "📝" }),
        "WorktreeCreate" => Some(EventMapping { status: AgentStatus::Running, emoji: "🌳" }),
        "WorktreeRemove" => Some(EventMapping { status: AgentStatus::Running, emoji: "🗑️" }),
        "PreCompact" => Some(EventMapping { status: AgentStatus::Running, emoji: "🗜️" }),
        "PostCompact" => Some(EventMapping { status: AgentStatus::Running, emoji: "📦" }),
        "SessionEnd" => Some(EventMapping { status: AgentStatus::Ended, emoji: "🏁" }),
        "Elicitation" => Some(EventMapping { status: AgentStatus::Permission, emoji: "❓" }),
        "ElicitationResult" => Some(EventMapping { status: AgentStatus::Running, emoji: "✅" }),
        _ => None,
    }
}

/// Extract a short detail string from tool_input based on the tool name.
/// For tools with file paths, formats as `file_path:line_number` (nvim-compatible) when a line number is available.
pub fn extract_tool_detail(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    let s = match tool_name {
        "Bash" => input.get("command")?.as_str()?,
        "Read" => {
            let path = input.get("file_path")?.as_str()?;
            if let Some(offset) = input.get("offset").and_then(|v| v.as_u64()) {
                return Some(format!("{path}:{offset}"));
            }
            path
        }
        "Write" | "Edit" => input.get("file_path")?.as_str()?,
        "NotebookEdit" => input.get("notebook_path")?.as_str()?,
        "Grep" => input.get("pattern")?.as_str()?,
        "Glob" => input.get("pattern")?.as_str()?,
        "WebFetch" => input.get("url")?.as_str()?,
        "WebSearch" => input.get("query")?.as_str()?,
        "Task" => {
            if let Some(desc) = input.get("description").and_then(|v| v.as_str()) {
                return Some(desc.to_string());
            }
            return None;
        }
        _ => return None,
    };
    Some(s.to_string())
}

/// Extract event-specific detail from ClaudeHookInput for non-tool events.
fn extract_event_detail(event: &str, input: &ClaudeHookInput) -> Option<String> {
    match event {
        "InstructionsLoaded" => input.instructions_file.clone(),
        "UserPromptExpansion" => input.expanded_text.clone(),
        "CwdChanged" => input.cwd.clone(),
        "FileChanged" => input.file_path.clone(),
        "PermissionRequest" | "PermissionDenied" => input.tool_name.clone(),
        "TaskCreated" | "TaskCompleted" => input
            .tool_input
            .as_ref()
            .and_then(|v| v.get("description"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "WorktreeCreate" | "WorktreeRemove" => input
            .tool_input
            .as_ref()
            .and_then(|v| v.get("path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

pub struct ClaudeHookParser;

impl HookParser for ClaudeHookParser {
    fn parse(&self, pane_id: String, raw_json: &str) -> Result<ParseResult> {
        let input: ClaudeHookInput = serde_json::from_str(raw_json.trim())
            .context("Failed to parse Claude Code hook input JSON from stdin")?;

        let event_name = input.hook_event_name.clone();

        // Special handling for Notification: check for permission_prompt
        let mapping = if event_name == "Notification" {
            if raw_json.contains("permission_prompt") {
                Some(EventMapping {
                    status: AgentStatus::Permission,
                    emoji: "🔐",
                })
            } else {
                // Non-permission notifications: suppress (don't update TUI state)
                return Ok(ParseResult {
                    state: AgentState::new(pane_id, AgentStatus::Running),
                    suppress: true,
                });
            }
        } else {
            claude_event_mapping(&event_name)
        };

        let mapping = match mapping {
            Some(m) => m,
            None => {
                // Unknown event: log and suppress
                eprintln!(
                    "[chikuwa hook] unknown event '{}', ignoring",
                    event_name
                );
                return Ok(ParseResult {
                    state: AgentState::new(pane_id, AgentStatus::Running),
                    suppress: true,
                });
            }
        };

        let mut state = AgentState::new(pane_id, mapping.status);
        state.session_id = input.session_id;
        state.agent_id = input.agent_id;
        state.hook_event_name = Some(event_name.clone());
        state.event_emoji = Some(mapping.emoji.to_string());

        // Extract tool info for PreToolUse events
        if let Some(ref name) = input.tool_name {
            let detail = input
                .tool_input
                .as_ref()
                .and_then(|inp| extract_tool_detail(name, inp));
            state.tools = vec![ToolInfo {
                name: name.clone(),
                detail,
            }];
        }

        state.tool_name = input.tool_name.clone();
        // For non-tool events, use event-specific detail
        state.tool_detail = extract_event_detail(&event_name, &input)
            .or_else(|| {
                input
                    .tool_name
                    .as_ref()
                    .and_then(|name| {
                        input
                            .tool_input
                            .as_ref()
                            .and_then(|inp| extract_tool_detail(name, inp))
                    })
            });

        Ok(ParseResult {
            state,
            suppress: false,
        })
    }
}

// ─── OpenCode Hook Parser ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenCodeHookInput {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    cwd: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

pub struct OpenCodeHookParser;

impl HookParser for OpenCodeHookParser {
    fn parse(&self, pane_id: String, raw_json: &str) -> Result<ParseResult> {
        let input: OpenCodeHookInput = serde_json::from_str(raw_json.trim())
            .context("Failed to parse OpenCode hook input JSON from stdin")?;

        let (status, emoji) = match input.event_type.as_str() {
            "file_edited" => (AgentStatus::Running, "📝"),
            "session_completed" => (AgentStatus::Ended, "🏁"),
            _ => {
                eprintln!(
                    "[chikuwa opencode-hook] unknown event type: {}",
                    input.event_type
                );
                return Ok(ParseResult {
                    state: AgentState::new(pane_id, AgentStatus::Running),
                    suppress: true,
                });
            }
        };

        let mut state = AgentState::new(pane_id, status);
        state.session_id = input.session_id;
        state.hook_event_name = Some(input.event_type);
        state.event_emoji = Some(emoji.to_string());

        if let Some(path) = input.file_path {
            state.tool_name = Some("edit".to_string());
            state.tool_detail = Some(path);
        }

        Ok(ParseResult {
            state,
            suppress: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_hook_input_deserialize() {
        let json = r#"{"hook_event_name":"SessionStart","session_id":"abc123"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(!result.suppress);
        assert_eq!(result.state.state, AgentStatus::Started);
        assert_eq!(result.state.event_emoji.as_deref(), Some("🚀"));
        assert_eq!(result.state.session_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_claude_hook_session_end() {
        let json = r#"{"hook_event_name":"SessionEnd"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(!result.suppress);
        assert_eq!(result.state.state, AgentStatus::Ended);
        assert_eq!(result.state.event_emoji.as_deref(), Some("🏁"));
    }

    #[test]
    fn test_claude_hook_pre_tool_use() {
        let json = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls -la"}}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(!result.suppress);
        assert_eq!(result.state.state, AgentStatus::Running);
        assert_eq!(result.state.event_emoji.as_deref(), Some("🔧"));
        assert_eq!(result.state.tools.len(), 1);
        assert_eq!(result.state.tools[0].name, "Bash");
        assert_eq!(result.state.tools[0].detail.as_deref(), Some("ls -la"));
    }

    #[test]
    fn test_claude_hook_notification_permission() {
        let json = r#"{"hook_event_name":"Notification","message":"permission_prompt foo"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(!result.suppress);
        assert_eq!(result.state.state, AgentStatus::Permission);
        assert_eq!(result.state.event_emoji.as_deref(), Some("🔐"));
    }

    #[test]
    fn test_claude_hook_notification_non_permission_suppressed() {
        let json = r#"{"hook_event_name":"Notification","message":"some info"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.suppress);
    }

    #[test]
    fn test_claude_hook_unknown_event_suppressed() {
        let json = r#"{"hook_event_name":"FutureEvent"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.suppress);
    }

    #[test]
    fn test_claude_hook_stop() {
        let json = r#"{"hook_event_name":"Stop"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.state.state, AgentStatus::Waiting);
        assert_eq!(result.state.event_emoji.as_deref(), Some("💤"));
    }

    #[test]
    fn test_claude_hook_stop_failure() {
        let json = r#"{"hook_event_name":"StopFailure"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.state.state, AgentStatus::Waiting);
        assert_eq!(result.state.event_emoji.as_deref(), Some("⚠️"));
    }

    #[test]
    fn test_claude_hook_all_events_have_mapping() {
        let events = [
            "SessionStart", "Setup", "InstructionsLoaded", "UserPromptSubmit",
            "UserPromptExpansion", "MessageDisplay", "PreToolUse", "PermissionRequest",
            "PostToolUse", "PostToolUseFailure", "PostToolBatch", "PermissionDenied",
            "Stop", "StopFailure", "SubagentStart", "SubagentStop",
            "TaskCreated", "TaskCompleted", "TeammateIdle", "ConfigChange",
            "CwdChanged", "FileChanged", "WorktreeCreate", "WorktreeRemove",
            "PreCompact", "PostCompact", "SessionEnd", "Elicitation", "ElicitationResult",
        ];
        for event in &events {
            assert!(claude_event_mapping(event).is_some(), "Missing mapping for event: {}", event);
        }
    }

    #[test]
    fn test_claude_hook_subagent_start() {
        let json = r#"{"hook_event_name":"SubagentStart","agent_id":"abc123","tool_name":"Task","tool_input":{"description":"Search codebase"}}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.state.state, AgentStatus::Running);
        assert_eq!(result.state.event_emoji.as_deref(), Some("🤖"));
        assert_eq!(result.state.agent_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_claude_hook_subagent_stop() {
        let json = r#"{"hook_event_name":"SubagentStop","agent_id":"abc123"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.state.state, AgentStatus::Ended);
        assert_eq!(result.state.event_emoji.as_deref(), Some("🏁"));
    }

    #[test]
    fn test_claude_hook_cwd_changed() {
        let json = r#"{"hook_event_name":"CwdChanged","cwd":"/home/user/project"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.state.state, AgentStatus::Running);
        assert_eq!(result.state.tool_detail.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn test_opencode_hook_file_edited() {
        let json = r#"{"type":"file_edited","file_path":"/src/main.rs","cwd":"/project"}"#;
        let parser = OpenCodeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(!result.suppress);
        assert_eq!(result.state.state, AgentStatus::Running);
        assert_eq!(result.state.event_emoji.as_deref(), Some("📝"));
    }

    #[test]
    fn test_opencode_hook_session_completed() {
        let json = r#"{"type":"session_completed","session_id":"sess-123"}"#;
        let parser = OpenCodeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.state.state, AgentStatus::Ended);
        assert_eq!(result.state.event_emoji.as_deref(), Some("🏁"));
    }

    #[test]
    fn test_extract_tool_detail_read_with_offset() {
        let input = serde_json::json!({"file_path": "/src/main.rs", "offset": 42});
        assert_eq!(
            extract_tool_detail("Read", &input),
            Some("/src/main.rs:42".to_string())
        );
    }

    #[test]
    fn test_extract_tool_detail_bash() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(
            extract_tool_detail("Bash", &input),
            Some("ls -la".to_string())
        );
    }

    #[test]
    fn test_extract_tool_detail_unknown() {
        let input = serde_json::json!({"foo": "bar"});
        assert_eq!(extract_tool_detail("UnknownTool", &input), None);
    }
}
```

- [ ] **Step 2: Update `src/agent/mod.rs` to export parser**

```rust
pub mod parser;
pub mod state;
pub mod subagent;

pub use parser::{ClaudeHookParser, HookParser, OpenCodeHookParser, ParseResult, extract_tool_detail};
pub use subagent::{SubagentInfo, SubagentStatus};
```

- [ ] **Step 3: Run tests**

Run: `cargo test agent::parser`
Expected: All parser tests pass

- [ ] **Step 4: Commit**

```bash
git add src/agent/parser.rs src/agent/mod.rs
git commit -m "feat: add HookParser trait with ClaudeHookParser and OpenCodeHookParser"
```

---

### Task 3: Refactor `hook.rs` to use `ClaudeHookParser`

**Files:**
- Modify: `src/hook.rs`

- [ ] **Step 1: Rewrite `hook.rs` to delegate to `ClaudeHookParser`**

Replace the entire `src/hook.rs` with:

```rust
use std::io::Read;

use anyhow::{Context, Result};

use crate::agent::{ClaudeHookParser, HookParser};
use crate::ipc;

/// Run the hook subcommand: read stdin JSON, parse via ClaudeHookParser, send state via IPC.
pub async fn run() -> Result<()> {
    let pane_id = std::env::var("TMUX_PANE")
        .context("TMUX_PANE environment variable not set (not running inside tmux?)")?;

    let mut stdin_buf = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin_buf)
        .context("Failed to read stdin")?;

    let parser = ClaudeHookParser;
    let result = parser.parse(pane_id, &stdin_buf)?;

    if result.suppress {
        return Ok(());
    }

    // Debug: log all events
    eprintln!(
        "[chikuwa hook] event: {} agent_id: {:?}",
        result.state.hook_event_name.as_deref().unwrap_or("?"),
        result.state.agent_id
    );

    ipc::broadcast_state(&result.state).await?;

    // Persist to JSONL so TUI can restore state on restart
    if let Err(e) = crate::persist::append_agent_state(&result.state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::AgentStatus;

    #[test]
    fn test_hook_run_requires_tmux_pane() {
        // Without TMUX_PANE, should fail
        std::env::remove_var("TMUX_PANE");
        // We can't easily test async run() without TMUX_PANE in unit tests,
        // but we can test the parser separately in parser tests.
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: All tests pass (hook tests are minimal; parser tests cover the logic)

- [ ] **Step 3: Commit**

```bash
git add src/hook.rs
git commit -m "refactor: simplify hook.rs to delegate to ClaudeHookParser"
```

---

### Task 4: Refactor `opencode.rs` to use `OpenCodeHookParser`

**Files:**
- Modify: `src/opencode.rs`

- [ ] **Step 1: Rewrite `opencode.rs` to delegate to `OpenCodeHookParser`**

Replace the entire `src/opencode.rs` with:

```rust
use std::io::Read;

use anyhow::{Context, Result};

use crate::agent::{HookParser, OpenCodeHookParser};
use crate::ipc;

/// Run the OpenCode hook subcommand: read stdin JSON, parse via OpenCodeHookParser, send state via IPC.
pub async fn run() -> Result<()> {
    let pane_id = std::env::var("TMUX_PANE")
        .context("TMUX_PANE environment variable not set (not running inside tmux?)")?;

    let mut stdin_buf = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin_buf)
        .context("Failed to read stdin")?;

    eprintln!("[chikuwa opencode-hook] received: {}", stdin_buf.trim());

    let parser = OpenCodeHookParser;
    let result = parser.parse(pane_id, &stdin_buf)?;

    if result.suppress {
        return Ok(());
    }

    ipc::broadcast_state(&result.state).await?;

    // Persist to JSONL so TUI can restore state on restart
    if let Err(e) = crate::persist::append_agent_state(&result.state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }

    Ok(())
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add src/opencode.rs
git commit -m "refactor: simplify opencode.rs to delegate to OpenCodeHookParser"
```

---

### Task 5: Fix `AgentState` construction sites across codebase

**Files:**
- Modify: `src/app.rs:1459-1523` (test: test_merge_subagent_state_new, test_merge_subagent_state_ended)
- Modify: `src/ui/tree.rs:1596-1608` (test: make_pane with AgentState)

- [ ] **Step 1: Add `event_emoji: None` to AgentState construction in `src/app.rs` tests**

In `src/app.rs`, find the two test functions `test_merge_subagent_state_new` and `test_merge_subagent_state_ended`. Add `event_emoji: None` to each `AgentState { ... }`:

```rust
// In test_merge_subagent_state_new (~line 1460):
let state = AgentState {
    tmux_pane: "%0".to_string(),
    session_id: None,
    agent_id: Some("abc123".to_string()),
    state: crate::agent::state::AgentStatus::Running,
    updated_at: 100,
    hook_event_name: Some("SubagentStart".to_string()),
    event_emoji: None,
    tool_name: Some("Task".to_string()),
    tool_detail: None,
    tools: vec![crate::agent::state::ToolInfo {
        name: "Task".to_string(),
        detail: Some("Search codebase".to_string()),
    }],
};

// In test_merge_subagent_state_ended - running_state (~line 1494):
let running_state = AgentState {
    tmux_pane: "%0".to_string(),
    session_id: None,
    agent_id: Some("abc123".to_string()),
    state: crate::agent::state::AgentStatus::Running,
    updated_at: 100,
    hook_event_name: None,
    event_emoji: None,
    tool_name: None,
    tool_detail: None,
    tools: vec![],
};

// In test_merge_subagent_state_ended - ended_state (~line 1507):
let ended_state = AgentState {
    tmux_pane: "%0".to_string(),
    session_id: None,
    agent_id: Some("abc123".to_string()),
    state: crate::agent::state::AgentStatus::Ended,
    updated_at: 200,
    hook_event_name: Some("SubagentStop".to_string()),
    event_emoji: None,
    tool_name: None,
    tool_detail: None,
    tools: vec![],
};
```

- [ ] **Step 2: Add `event_emoji: None` to AgentState in `src/ui/tree.rs` tests**

In `src/ui/tree.rs`, find `make_pane` function in tests and the `AgentState { ... }` construction inside `make_sessions()` (~line 1597). Add `event_emoji: None`:

```rust
Some(AgentState {
    tmux_pane: "%0".to_string(),
    session_id: None,
    agent_id: None,
    state: AgentStatus::Running,
    updated_at: 100,
    hook_event_name: None,
    event_emoji: None,
    tool_name: None,
    tool_detail: None,
    tools: Vec::new(),
}),
```

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/app.rs src/ui/tree.rs
git commit -m "fix: add event_emoji field to AgentState test constructions"
```

---

### Task 6: Display emoji in the TUI status sub-lines

**Files:**
- Modify: `src/ui/tree.rs:990-1006` (render_bordered_agent_status_sub_lines)

- [ ] **Step 1: Update status rendering to show event emoji**

In `src/ui/tree.rs`, in the `render_bordered_agent_status_sub_lines` function, replace the status line construction (around line 999-1006) to include the emoji:

Find this code:
```rust
    // Status line
    let mut status_spans = vec![
        Span::styled(prefix.to_string(), Style::default().fg(theme::COLOR_PURPLE)),
        Span::styled(
            theme::status_icon(&agent.state, anim_frame).to_string(),
            theme::status_style(&agent.state, session_attached),
        ),
        Span::styled(format!(" {}", status_label), dim_style),
    ];
```

Replace with:
```rust
    // Status line
    let mut status_spans = vec![
        Span::styled(prefix.to_string(), Style::default().fg(theme::COLOR_PURPLE)),
        Span::styled(
            theme::status_icon(&agent.state, anim_frame).to_string(),
            theme::status_style(&agent.state, session_attached),
        ),
        Span::styled(format!(" {}", status_label), dim_style),
    ];
    // Add event emoji if available
    if let Some(ref emoji) = agent.event_emoji {
        status_spans.push(Span::styled(format!(" {}", emoji), dim_style));
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add src/ui/tree.rs
git commit -m "feat: display event emoji in TUI agent status sub-lines"
```

---

### Task 7: Format and lint

**Files:**
- All modified files

- [ ] **Step 1: Run `cargo fmt`**

Run: `cargo fmt`

- [ ] **Step 2: Run `cargo clippy`**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run `cargo test`**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit if any formatting changes**

```bash
git add -A
git commit -m "style: format code"
```
