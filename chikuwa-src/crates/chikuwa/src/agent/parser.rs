use anyhow::{Context, Result};
use serde::Deserialize;

use super::claude::ClaudeState;
use super::codex_state::CodexState;
use super::opencode_state::OpenCodeState;
use super::state::{ActiveTool, AgentData, AgentState, AgentStatus, ToolKey};

/// How the TUI should display a parsed event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    /// Normal display — update state, emoji, tools, everything
    Show,
    /// Update tools list only — don't change visual state (emoji, status, tool_name)
    Silent,
    /// Ignore completely — don't send to TUI
    Suppress,
}

/// Result of parsing a hook event.
pub struct ParseResult {
    pub state: AgentState,
    /// How the TUI should display this event.
    pub display: DisplayMode,
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
    #[allow(dead_code)]
    tool_response: Option<serde_json::Value>,
    #[serde(default)]
    tool_use_id: Option<String>,
    /// PostToolUseFailure: error description string.
    #[serde(default)]
    error: Option<String>,
    /// Notification events: message content.
    #[serde(default)]
    #[allow(dead_code)]
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

/// Event mapping entry: (status, emoji)
struct EventMapping {
    status: AgentStatus,
    emoji: &'static str,
}

/// Get the event mapping for a Claude Code hook event name.
fn claude_event_mapping(event: &str) -> Option<EventMapping> {
    match event {
        "SessionStart" => Some(EventMapping {
            status: AgentStatus::Started,
            emoji: "🚀",
        }),
        "Setup" => Some(EventMapping {
            status: AgentStatus::Started,
            emoji: "⚙️",
        }),
        "InstructionsLoaded" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "📄",
        }),
        "UserPromptSubmit" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "💭",
        }),
        "UserPromptExpansion" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "⚡",
        }),
        "MessageDisplay" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "💬",
        }),
        "PreToolUse" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🔧",
        }),
        "PermissionRequest" => Some(EventMapping {
            status: AgentStatus::Permission,
            emoji: "🔐",
        }),
        "PostToolUse" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "✅",
        }),
        "PostToolUseFailure" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "❌",
        }),
        "PostToolBatch" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "📦",
        }),
        "PermissionDenied" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🚫",
        }),
        "Notification" => None, // handled specially
        "Stop" => Some(EventMapping {
            status: AgentStatus::Waiting,
            emoji: "💤",
        }),
        "StopFailure" => Some(EventMapping {
            status: AgentStatus::Waiting,
            emoji: "⚠️",
        }),
        "SubagentStart" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🤖",
        }),
        "SubagentStop" => Some(EventMapping {
            status: AgentStatus::Ended,
            emoji: "🏁",
        }),
        "TaskCreated" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "📋",
        }),
        "TaskCompleted" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "✔️",
        }),
        "TeammateIdle" => Some(EventMapping {
            status: AgentStatus::Waiting,
            emoji: "👥",
        }),
        "ConfigChange" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "⚙️",
        }),
        "CwdChanged" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "📁",
        }),
        "FileChanged" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "📝",
        }),
        "WorktreeCreate" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🌳",
        }),
        "WorktreeRemove" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🗑️",
        }),
        "PreCompact" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🗜️",
        }),
        "PostCompact" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "📦",
        }),
        "SessionEnd" => Some(EventMapping {
            status: AgentStatus::Ended,
            emoji: "🏁",
        }),
        "Elicitation" => Some(EventMapping {
            status: AgentStatus::Permission,
            emoji: "❓",
        }),
        "ElicitationResult" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "✅",
        }),
        _ => None,
    }
}

/// Extract a short detail string from tool_input based on the tool name.
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
                // Non-permission notifications: suppress
                return Ok(ParseResult {
                    state: AgentState::new(
                        pane_id,
                        AgentData::Claude(ClaudeState {
                            session_id: input.session_id,
                            agent_id: input.agent_id,
                            status: AgentStatus::Running,
                            hook_event_name: event_name,
                            event_emoji: "💬".to_string(),
                            tool_name: None,
                            tool_detail: None,
                            active_tools: Vec::new(),
                            recent_tools: Vec::new(),
                            failure_detail: None,
                        }),
                    ),
                    display: DisplayMode::Suppress,
                });
            }
        } else {
            claude_event_mapping(&event_name)
        };

        let mapping = match mapping {
            Some(m) => m,
            None => {
                eprintln!("[chikuwa hook] unknown event '{}', ignoring", event_name);
                return Ok(ParseResult {
                    state: AgentState::new(
                        pane_id,
                        AgentData::Claude(ClaudeState {
                            session_id: input.session_id,
                            agent_id: input.agent_id,
                            status: AgentStatus::Running,
                            hook_event_name: event_name,
                            event_emoji: "❓".to_string(),
                            tool_name: None,
                            tool_detail: None,
                            active_tools: Vec::new(),
                            recent_tools: Vec::new(),
                            failure_detail: None,
                        }),
                    ),
                    display: DisplayMode::Suppress,
                });
            }
        };

        // Determine display mode based on event type
        let display = match event_name.as_str() {
            "PostToolUse" => DisplayMode::Silent,
            "PostToolUseFailure" => DisplayMode::Show,
            _ => DisplayMode::Show,
        };

        // Extract tool_detail
        let tool_detail = extract_event_detail(&event_name, &input).or_else(|| {
            input.tool_name.as_ref().and_then(|name| {
                input
                    .tool_input
                    .as_ref()
                    .and_then(|inp| extract_tool_detail(name, inp))
            })
        });

        // Build active_tools for this event
        let active_tools = if let Some(ref name) = input.tool_name {
            let tool_use_id = input.tool_use_id.clone().unwrap_or_else(|| {
                // Fallback: generate a synthetic key from name + input hash
                format!(
                    "{}:{:x}",
                    name,
                    input
                        .tool_input
                        .as_ref()
                        .map(|v| v.to_string().len())
                        .unwrap_or(0)
                )
            });
            vec![ActiveTool {
                key: ToolKey::Claude { tool_use_id },
                name: name.clone(),
                detail: tool_detail.clone(),
                failure_detail: None,
            }]
        } else {
            Vec::new()
        };

        // Extract failure detail for PostToolUseFailure
        let failure_detail = if event_name == "PostToolUseFailure" {
            input
                .error
                .as_deref()
                .filter(|m| !m.is_empty())
                .map(|m| {
                    if m.chars().count() > 80 {
                        format!("{}...", m.chars().take(77).collect::<String>())
                    } else {
                        m.to_string()
                    }
                })
                .or_else(|| input.tool_name.as_ref().map(|n| format!("{} failed", n)))
        } else {
            None
        };

        let claude_state = ClaudeState {
            session_id: input.session_id,
            agent_id: input.agent_id,
            status: mapping.status,
            hook_event_name: event_name,
            event_emoji: mapping.emoji.to_string(),
            tool_name: input.tool_name,
            tool_detail,
            active_tools,
            recent_tools: Vec::new(),
            failure_detail,
        };

        let agent_state = AgentState::new(pane_id, AgentData::Claude(claude_state));

        Ok(ParseResult {
            state: agent_state,
            display,
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
                let opencode_state = OpenCodeState {
                    session_id: input.session_id,
                    status: AgentStatus::Running,
                    event_type: Some(input.event_type),
                    event_emoji: None,
                    tool_name: None,
                    tool_detail: None,
                    active_tools: Vec::new(),
                    recent_tools: Vec::new(),
                    is_busy: false,
                };
                return Ok(ParseResult {
                    state: AgentState::new(pane_id, AgentData::OpenCode(opencode_state)),
                    display: DisplayMode::Suppress,
                });
            }
        };

        let mut active_tools = Vec::new();
        let mut tool_name = None;
        let mut tool_detail = None;

        if let Some(path) = input.file_path {
            let key = ToolKey::OpenCode {
                name: "edit".to_string(),
                detail: Some(path.clone()),
            };
            active_tools.push(ActiveTool {
                key,
                name: "edit".to_string(),
                detail: Some(path.clone()),
                failure_detail: None,
            });
            tool_name = Some("edit".to_string());
            tool_detail = Some(path);
        }

        let opencode_state = OpenCodeState {
            session_id: input.session_id,
            status,
            event_type: Some(input.event_type),
            event_emoji: Some(emoji.to_string()),
            tool_name,
            tool_detail,
            active_tools,
            recent_tools: Vec::new(),
            is_busy: status == AgentStatus::Running,
        };

        Ok(ParseResult {
            state: AgentState::new(pane_id, AgentData::OpenCode(opencode_state)),
            display: DisplayMode::Show,
        })
    }
}

// ─── Codex CLI Hook Parser ──────────────────────────────────────────

/// Input JSON from Codex CLI hooks (stdin).
#[derive(Debug, Deserialize)]
struct CodexHookInput {
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    tool_input: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    tool_response: Option<serde_json::Value>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    agent_transcript_path: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    stop_hook_active: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    last_assistant_message: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    prompt: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    trigger: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    source: Option<String>,
}

fn codex_event_mapping(event: &str) -> Option<EventMapping> {
    match event {
        "SessionStart" => Some(EventMapping {
            status: AgentStatus::Started,
            emoji: "🚀",
        }),
        "SubagentStart" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🤖",
        }),
        "PreToolUse" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🔧",
        }),
        "PermissionRequest" => Some(EventMapping {
            status: AgentStatus::Permission,
            emoji: "🔐",
        }),
        "PostToolUse" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "✅",
        }),
        "PreCompact" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "🗜️",
        }),
        "PostCompact" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "📦",
        }),
        "UserPromptSubmit" => Some(EventMapping {
            status: AgentStatus::Running,
            emoji: "💭",
        }),
        "SubagentStop" => Some(EventMapping {
            status: AgentStatus::Ended,
            emoji: "🏁",
        }),
        "Stop" => Some(EventMapping {
            status: AgentStatus::Waiting,
            emoji: "💤",
        }),
        _ => None,
    }
}

fn extract_codex_tool_detail(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    match tool_name {
        "Bash" | "apply_patch" => input
            .get("command")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        _ if tool_name.starts_with("mcp__") => {
            let short_name = tool_name.rsplit("__").next().unwrap_or(tool_name);
            for key in ["file_path", "path", "command", "pattern", "query", "url"] {
                if let Some(value) = input.get(key).and_then(|value| value.as_str()) {
                    return Some(format!("{short_name}({value})"));
                }
            }
            Some(short_name.to_string())
        }
        _ => None,
    }
}

pub struct CodexHookParser;

impl HookParser for CodexHookParser {
    fn parse(&self, pane_id: String, raw_json: &str) -> Result<ParseResult> {
        let input: CodexHookInput = serde_json::from_str(raw_json.trim())
            .context("Failed to parse Codex CLI hook input JSON from stdin")?;

        let event_name = input.hook_event_name.clone();
        let Some(mapping) = codex_event_mapping(&event_name) else {
            return Ok(ParseResult {
                state: AgentState::new(
                    pane_id,
                    AgentData::Codex(CodexState::new(&event_name, AgentStatus::Waiting, "💤")),
                ),
                display: DisplayMode::Suppress,
            });
        };

        let tool_name = input.tool_name.clone();
        let tool_detail = tool_name.as_ref().and_then(|name| {
            input
                .tool_input
                .as_ref()
                .and_then(|tool_input| extract_codex_tool_detail(name, tool_input))
        });

        let active_tools = match (
            event_name.as_str(),
            tool_name.as_ref(),
            input.tool_use_id.as_ref(),
        ) {
            ("PreToolUse" | "PostToolUse", Some(name), Some(tool_use_id)) => vec![ActiveTool {
                key: ToolKey::Codex {
                    tool_use_id: tool_use_id.clone(),
                },
                name: name.clone(),
                detail: tool_detail.clone(),
                failure_detail: None,
            }],
            _ => Vec::new(),
        };

        let state = CodexState {
            session_id: input.session_id,
            agent_id: input.agent_id,
            status: mapping.status,
            hook_event_name: event_name,
            event_emoji: mapping.emoji.to_string(),
            tool_name,
            tool_detail,
            active_tools,
            recent_tools: Vec::new(),
            failure_detail: None,
            turn_id: input.turn_id,
            permission_mode: input.permission_mode,
            model: input.model,
            cwd: input.cwd,
            agent_type: input.agent_type,
            transcript_path: input.agent_transcript_path.or(input.transcript_path),
        };

        Ok(ParseResult {
            state: AgentState::new(pane_id, AgentData::Codex(state)),
            display: DisplayMode::Show,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::AgentView;

    fn claude_data(result: &ParseResult) -> &ClaudeState {
        match &result.state.data {
            AgentData::Claude(c) => c,
            _ => panic!("expected Claude data"),
        }
    }

    fn opencode_data(result: &ParseResult) -> &OpenCodeState {
        match &result.state.data {
            AgentData::OpenCode(o) => o,
            _ => panic!("expected OpenCode data"),
        }
    }

    fn codex_input(event: &str) -> String {
        format!(
            r#"{{"hook_event_name":"{}","session_id":"sess-1","cwd":"/repo","turn_id":"turn-1","model":"o3","transcript_path":"/tmp/transcript.jsonl"}}"#,
            event
        )
    }

    #[test]
    fn test_claude_hook_input_deserialize() {
        let json = r#"{"hook_event_name":"SessionStart","session_id":"abc123"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.display == DisplayMode::Show);
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Started);
        assert_eq!(data.event_emoji, "🚀");
        assert_eq!(data.session_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_claude_hook_session_end() {
        let json = r#"{"hook_event_name":"SessionEnd"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.display == DisplayMode::Show);
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Ended);
        assert_eq!(data.event_emoji, "🏁");
    }

    #[test]
    fn test_claude_hook_pre_tool_use() {
        let json = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls -la"},"tool_use_id":"toolu_01ABC"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.display == DisplayMode::Show);
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Running);
        assert_eq!(data.event_emoji, "🔧");
        assert_eq!(data.active_tools.len(), 1);
        assert_eq!(data.active_tools[0].name, "Bash");
        assert_eq!(data.active_tools[0].detail.as_deref(), Some("ls -la"));
        assert_eq!(
            data.active_tools[0].key,
            ToolKey::Claude {
                tool_use_id: "toolu_01ABC".to_string()
            }
        );
    }

    #[test]
    fn test_claude_hook_notification_permission() {
        let json = r#"{"hook_event_name":"Notification","message":"permission_prompt foo"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.display == DisplayMode::Show);
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Permission);
        assert_eq!(data.event_emoji, "🔐");
    }

    #[test]
    fn test_claude_hook_notification_non_permission_suppressed() {
        let json = r#"{"hook_event_name":"Notification","message":"some info"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.display == DisplayMode::Suppress);
    }

    #[test]
    fn test_claude_hook_unknown_event_suppressed() {
        let json = r#"{"hook_event_name":"FutureEvent"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.display == DisplayMode::Suppress);
    }

    #[test]
    fn test_claude_hook_stop() {
        let json = r#"{"hook_event_name":"Stop"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Waiting);
        assert_eq!(data.event_emoji, "💤");
    }

    #[test]
    fn test_claude_hook_stop_failure() {
        let json = r#"{"hook_event_name":"StopFailure"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Waiting);
        assert_eq!(data.event_emoji, "⚠️");
    }

    #[test]
    fn test_claude_hook_all_events_have_mapping() {
        let events = [
            "SessionStart",
            "Setup",
            "InstructionsLoaded",
            "UserPromptSubmit",
            "UserPromptExpansion",
            "MessageDisplay",
            "PreToolUse",
            "PermissionRequest",
            "PostToolUse",
            "PostToolUseFailure",
            "PostToolBatch",
            "PermissionDenied",
            "Stop",
            "StopFailure",
            "SubagentStart",
            "SubagentStop",
            "TaskCreated",
            "TaskCompleted",
            "TeammateIdle",
            "ConfigChange",
            "CwdChanged",
            "FileChanged",
            "WorktreeCreate",
            "WorktreeRemove",
            "PreCompact",
            "PostCompact",
            "SessionEnd",
            "Elicitation",
            "ElicitationResult",
        ];
        for event in &events {
            assert!(
                claude_event_mapping(event).is_some(),
                "Missing mapping for event: {}",
                event
            );
        }
    }

    #[test]
    fn test_claude_hook_subagent_start() {
        let json = r#"{"hook_event_name":"SubagentStart","agent_id":"abc123","tool_name":"Task","tool_input":{"description":"Search codebase"}}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Running);
        assert_eq!(data.event_emoji, "🤖");
        assert_eq!(data.agent_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_claude_hook_subagent_stop() {
        let json = r#"{"hook_event_name":"SubagentStop","agent_id":"abc123"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Ended);
        assert_eq!(data.event_emoji, "🏁");
    }

    #[test]
    fn test_claude_hook_cwd_changed() {
        let json = r#"{"hook_event_name":"CwdChanged","cwd":"/home/user/project"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Running);
        assert_eq!(data.tool_detail.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn test_claude_hook_post_tool_use_silent() {
        let json = r#"{"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"ls"},"tool_use_id":"toolu_01"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.display, DisplayMode::Silent);
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Running);
        assert!(data.failure_detail.is_none());
    }

    #[test]
    fn test_claude_hook_post_tool_use_failure_with_error() {
        let json = r#"{"hook_event_name":"PostToolUseFailure","tool_name":"Bash","error":"command failed with exit code 1","tool_use_id":"toolu_02"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert_eq!(result.display, DisplayMode::Show);
        let data = claude_data(&result);
        assert_eq!(data.status, AgentStatus::Running);
        assert_eq!(
            data.failure_detail.as_deref(),
            Some("command failed with exit code 1")
        );
    }

    #[test]
    fn test_claude_hook_post_tool_use_failure_fallback() {
        let json = r#"{"hook_event_name":"PostToolUseFailure","tool_name":"Read","tool_use_id":"toolu_03"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = claude_data(&result);
        assert_eq!(data.failure_detail.as_deref(), Some("Read failed"));
    }

    #[test]
    fn test_claude_hook_post_tool_use_failure_truncation() {
        let long_msg = "x".repeat(100);
        let json = format!(
            r#"{{"hook_event_name":"PostToolUseFailure","tool_name":"Bash","error":"{}","tool_use_id":"toolu_04"}}"#,
            long_msg
        );
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), &json).unwrap();
        let detail = claude_data(&result).failure_detail.clone().unwrap();
        assert!(detail.ends_with("..."));
    }

    #[test]
    fn test_claude_hook_post_tool_use_failure_utf8_truncation() {
        let long_msg = "日本語".repeat(30);
        let json = format!(
            r#"{{"hook_event_name":"PostToolUseFailure","tool_name":"Bash","error":"{}","tool_use_id":"toolu_05"}}"#,
            long_msg
        );
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), &json).unwrap();
        assert!(claude_data(&result).failure_detail.is_some());
        assert!(claude_data(&result)
            .failure_detail
            .as_ref()
            .unwrap()
            .ends_with("..."));
    }

    #[test]
    fn test_claude_hook_tool_use_id_in_active_tool() {
        let json = r#"{"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"/src/main.rs","offset":42},"tool_use_id":"toolu_ABC123"}"#;
        let parser = ClaudeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = claude_data(&result);
        assert_eq!(data.active_tools.len(), 1);
        assert_eq!(
            data.active_tools[0].key,
            ToolKey::Claude {
                tool_use_id: "toolu_ABC123".to_string()
            }
        );
    }

    #[test]
    fn test_opencode_hook_file_edited() {
        let json = r#"{"type":"file_edited","file_path":"/src/main.rs","cwd":"/project"}"#;
        let parser = OpenCodeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        assert!(result.display == DisplayMode::Show);
        let data = opencode_data(&result);
        assert_eq!(data.status, AgentStatus::Running);
        assert_eq!(data.event_emoji.as_deref(), Some("📝"));
    }

    #[test]
    fn test_opencode_hook_session_completed() {
        let json = r#"{"type":"session_completed","session_id":"sess-123"}"#;
        let parser = OpenCodeHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();
        let data = opencode_data(&result);
        assert_eq!(data.status, AgentStatus::Ended);
        assert_eq!(data.event_emoji.as_deref(), Some("🏁"));
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

    #[test]
    fn test_codex_hook_session_start() {
        let parser = CodexHookParser;
        let result = parser
            .parse("%0".to_string(), &codex_input("SessionStart"))
            .unwrap();

        assert_eq!(result.display, DisplayMode::Show);
        assert_eq!(result.state.status(), AgentStatus::Started);
        assert_eq!(
            result.state.source(),
            crate::agent::state::AgentSource::Codex
        );
        assert_eq!(result.state.event_emoji(), Some("🚀"));
    }

    #[test]
    fn test_codex_hook_pre_tool_use_bash() {
        let json = r#"{"hook_event_name":"PreToolUse","session_id":"sess-1","turn_id":"turn-1","tool_name":"Bash","tool_use_id":"call-1","tool_input":{"command":"ls -la"}}"#;
        let parser = CodexHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();

        assert_eq!(result.state.status(), AgentStatus::Running);
        assert_eq!(result.state.current_tool_name(), Some("Bash"));
        assert_eq!(result.state.current_tool_detail(), Some("ls -la"));
        assert_eq!(result.state.active_tools().len(), 1);
    }

    #[test]
    fn test_codex_hook_post_tool_use_contains_removal_tool() {
        let json = r#"{"hook_event_name":"PostToolUse","session_id":"sess-1","turn_id":"turn-1","tool_name":"Bash","tool_use_id":"call-1","tool_input":{"command":"ls -la"},"tool_response":{"exit_code":0}}"#;
        let parser = CodexHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();

        assert_eq!(result.state.status(), AgentStatus::Running);
        assert_eq!(result.state.active_tools().len(), 1);
        assert_eq!(
            result.state.active_tools()[0].key,
            ToolKey::Codex {
                tool_use_id: "call-1".to_string()
            }
        );
    }

    #[test]
    fn test_codex_hook_permission_request() {
        let json = r#"{"hook_event_name":"PermissionRequest","session_id":"sess-1","turn_id":"turn-1","tool_name":"Bash","tool_input":{"command":"rm -rf /","description":"Delete everything"}}"#;
        let parser = CodexHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();

        assert_eq!(result.state.status(), AgentStatus::Permission);
        assert_eq!(result.state.event_emoji(), Some("🔐"));
        assert_eq!(result.state.current_tool_detail(), Some("rm -rf /"));
    }

    #[test]
    fn test_codex_hook_subagent_start_and_stop() {
        let start = r#"{"hook_event_name":"SubagentStart","session_id":"sess-1","turn_id":"turn-1","agent_id":"agent-1","agent_type":"code-review"}"#;
        let stop = r#"{"hook_event_name":"SubagentStop","session_id":"sess-1","turn_id":"turn-1","agent_id":"agent-1","agent_type":"code-review","agent_transcript_path":"/tmp/agent.jsonl"}"#;
        let parser = CodexHookParser;

        let start_result = parser.parse("%0".to_string(), start).unwrap();
        let stop_result = parser.parse("%0".to_string(), stop).unwrap();

        assert_eq!(start_result.state.agent_id(), Some("agent-1"));
        assert_eq!(start_result.state.status(), AgentStatus::Running);
        assert_eq!(stop_result.state.agent_id(), Some("agent-1"));
        assert_eq!(stop_result.state.status(), AgentStatus::Ended);
    }

    #[test]
    fn test_codex_hook_stop() {
        let json = r#"{"hook_event_name":"Stop","session_id":"sess-1","turn_id":"turn-1","stop_hook_active":false,"last_assistant_message":"Done"}"#;
        let parser = CodexHookParser;
        let result = parser.parse("%0".to_string(), json).unwrap();

        assert_eq!(result.state.status(), AgentStatus::Waiting);
        assert_eq!(result.state.event_emoji(), Some("💤"));
    }

    #[test]
    fn test_codex_hook_unknown_event_suppressed() {
        let parser = CodexHookParser;
        let result = parser
            .parse("%0".to_string(), &codex_input("UnknownEvent"))
            .unwrap();

        assert_eq!(result.display, DisplayMode::Suppress);
    }

    #[test]
    fn test_codex_hook_all_events_have_mapping() {
        let events = [
            "SessionStart",
            "SubagentStart",
            "PreToolUse",
            "PermissionRequest",
            "PostToolUse",
            "PreCompact",
            "PostCompact",
            "UserPromptSubmit",
            "SubagentStop",
            "Stop",
        ];
        let parser = CodexHookParser;

        for event in events {
            let result = parser.parse("%0".to_string(), &codex_input(event)).unwrap();
            assert_eq!(
                result.display,
                DisplayMode::Show,
                "{event} should be displayed"
            );
        }
    }

    #[test]
    fn test_extract_codex_tool_detail_apply_patch() {
        let input = serde_json::json!({"command": "apply patch content"});

        assert_eq!(
            extract_codex_tool_detail("apply_patch", &input),
            Some("apply patch content".to_string())
        );
    }

    #[test]
    fn test_extract_codex_tool_detail_mcp_path() {
        let input = serde_json::json!({"file_path": "/tmp/test.txt"});
        let detail = extract_codex_tool_detail("mcp__fs__read", &input).unwrap();

        assert!(detail.contains("read"));
        assert!(detail.contains("/tmp/test.txt"));
    }
}
