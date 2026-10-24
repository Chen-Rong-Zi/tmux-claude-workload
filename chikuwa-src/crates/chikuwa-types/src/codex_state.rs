use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::state::{push_recent_tool, ActiveTool, AgentStatus};

/// Full state from Codex CLI hooks.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodexState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub status: AgentStatus,
    pub hook_event_name: String,
    pub event_emoji: String,
    /// Currently running tool (from the latest event)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_detail: Option<String>,
    /// All active (in-flight) tool calls
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_tools: Vec<ActiveTool>,
    /// Recently completed tool calls, oldest to newest
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_tools: Vec<ActiveTool>,
    /// Failure message from PostToolUse failure
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
}

impl CodexState {
    pub fn new(event: &str, status: AgentStatus, emoji: &str) -> Self {
        Self {
            session_id: None,
            agent_id: None,
            status,
            hook_event_name: event.to_string(),
            event_emoji: emoji.to_string(),
            tool_name: None,
            tool_detail: None,
            active_tools: Vec::new(),
            recent_tools: Vec::new(),
            failure_detail: None,
            turn_id: None,
            permission_mode: None,
            model: None,
            cwd: None,
            agent_type: None,
            transcript_path: None,
        }
    }

    pub fn merge(incoming: CodexState, existing: &CodexState) -> CodexState {
        let event = incoming.hook_event_name.clone();
        let is_silent = event == "PostToolUse";

        let session_id = incoming
            .session_id
            .clone()
            .or_else(|| existing.session_id.clone());
        let agent_id = incoming
            .agent_id
            .clone()
            .or_else(|| existing.agent_id.clone());

        let mut recent_tools = existing.recent_tools.clone();

        let active_tools = if incoming.status == AgentStatus::Running {
            match event.as_str() {
                "PreToolUse" => {
                    let mut tools = existing.active_tools.clone();
                    for tool in &incoming.active_tools {
                        tools.push(tool.clone());
                    }
                    tools
                }
                "PostToolUse" => {
                    let mut tools = existing.active_tools.clone();
                    if let Some(removing) = incoming.active_tools.first() {
                        let pos = tools
                            .iter()
                            .position(|tool| tool.key == removing.key)
                            .or_else(|| tools.iter().position(|tool| tool.name == removing.name));
                        if let Some(pos) = pos {
                            let completed = tools.remove(pos);
                            push_recent_tool(&mut recent_tools, completed);
                        }
                    }
                    tools
                }
                _ => existing.active_tools.clone(),
            }
        } else {
            // Preserve active tools as recent to avoid visual flicker on Stop/Waiting
            for tool in &existing.active_tools {
                push_recent_tool(&mut recent_tools, tool.clone());
            }
            Vec::new()
        };
        let mut merged = incoming;
        merged.session_id = session_id;
        merged.agent_id = agent_id;
        merged.active_tools = active_tools;
        merged.recent_tools = recent_tools;

        if is_silent {
            merged.event_emoji = existing.event_emoji.clone();
            merged.hook_event_name = existing.hook_event_name.clone();
            merged.tool_name = existing.tool_name.clone();
            merged.tool_detail = existing.tool_detail.clone();
            merged.status = existing.status;
            merged.failure_detail = existing.failure_detail.clone();
        }

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ActiveTool, AgentStatus, ToolKey};

    fn make_state(
        event: &str,
        status: AgentStatus,
        tool_name: Option<&str>,
        tool_use_id: Option<&str>,
    ) -> CodexState {
        let mut state = CodexState::new(event, status, "🔧");
        state.tool_name = tool_name.map(str::to_string);
        if let (Some(name), Some(id)) = (tool_name, tool_use_id) {
            state.active_tools.push(ActiveTool {
                key: ToolKey::Codex {
                    tool_use_id: id.to_string(),
                },
                name: name.to_string(),
                detail: None,
                failure_detail: None,
            });
        }
        state
    }

    #[test]
    fn test_codex_state_new() {
        let state = CodexState::new("SessionStart", AgentStatus::Started, "🚀");
        assert_eq!(state.hook_event_name, "SessionStart");
        assert_eq!(state.status, AgentStatus::Started);
        assert_eq!(state.event_emoji, "🚀");
    }

    #[test]
    fn test_session_id_preserved_when_incoming_missing() {
        let incoming = make_state("Stop", AgentStatus::Waiting, None, None);
        let mut existing = make_state("SessionStart", AgentStatus::Started, None, None);
        existing.session_id = Some("sess-1".to_string());

        let merged = CodexState::merge(incoming, &existing);

        assert_eq!(merged.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn test_agent_id_preserved_when_incoming_missing() {
        let incoming = make_state("Stop", AgentStatus::Waiting, None, None);
        let mut existing = make_state("SubagentStart", AgentStatus::Running, None, None);
        existing.agent_id = Some("agent-1".to_string());

        let merged = CodexState::merge(incoming, &existing);

        assert_eq!(merged.agent_id.as_deref(), Some("agent-1"));
    }

    #[test]
    fn test_pre_tool_use_adds_active_tool() {
        let incoming = make_state(
            "PreToolUse",
            AgentStatus::Running,
            Some("Bash"),
            Some("call-1"),
        );
        let existing = make_state("SessionStart", AgentStatus::Started, None, None);

        let merged = CodexState::merge(incoming, &existing);

        assert_eq!(merged.active_tools.len(), 1);
        assert_eq!(
            merged.active_tools[0].key,
            ToolKey::Codex {
                tool_use_id: "call-1".to_string()
            }
        );
    }

    #[test]
    fn test_post_tool_use_removes_matching_tool_and_preserves_visual_state() {
        let mut existing = make_state(
            "PreToolUse",
            AgentStatus::Running,
            Some("Bash"),
            Some("call-1"),
        );
        existing.tool_detail = Some("ls".to_string());
        let incoming = make_state(
            "PostToolUse",
            AgentStatus::Running,
            Some("Bash"),
            Some("call-1"),
        );

        let merged = CodexState::merge(incoming, &existing);

        assert!(merged.active_tools.is_empty());
        assert_eq!(merged.recent_tools.len(), 1);
        assert_eq!(merged.recent_tools[0].name, "Bash");
        assert_eq!(merged.hook_event_name, "PreToolUse");
        assert_eq!(merged.tool_detail.as_deref(), Some("ls"));
    }

    #[test]
    fn test_non_running_status_clears_active_tools() {
        let existing = make_state(
            "PreToolUse",
            AgentStatus::Running,
            Some("Bash"),
            Some("call-1"),
        );
        let incoming = make_state("Stop", AgentStatus::Waiting, None, None);

        let merged = CodexState::merge(incoming, &existing);

        assert!(merged.active_tools.is_empty());
        // recent_tools preserved across Stop/Waiting transitions to avoid visual flicker
        assert_eq!(merged.recent_tools.len(), 1);
        assert_eq!(merged.recent_tools[0].name, "Bash");
    }

    #[test]
    fn test_serialization_roundtrip_preserves_codex_fields() {
        let mut state = make_state(
            "PreToolUse",
            AgentStatus::Running,
            Some("Bash"),
            Some("call-1"),
        );
        state.session_id = Some("sess-1".to_string());
        state.turn_id = Some("turn-1".to_string());
        state.permission_mode = Some("default".to_string());
        state.model = Some("o3".to_string());
        state.cwd = Some("/repo".to_string());

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: CodexState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.session_id.as_deref(), Some("sess-1"));
        assert_eq!(deserialized.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(deserialized.permission_mode.as_deref(), Some("default"));
        assert_eq!(deserialized.model.as_deref(), Some("o3"));
        assert_eq!(deserialized.cwd.as_deref(), Some("/repo"));
    }
}
