use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::state::{push_recent_tool, ActiveTool, AgentStatus};

/// Full state from Claude Code hooks.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeState {
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
    /// Failure message from PostToolUseFailure
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
}

impl ClaudeState {
    /// Merge an incoming event into existing state, returning the new state.
    pub fn merge(incoming: ClaudeState, existing: &ClaudeState) -> ClaudeState {
        let event = incoming.hook_event_name.clone();
        let is_silent = event == "PostToolUse";

        // Preserve session_id if incoming is None
        let session_id = incoming
            .session_id
            .clone()
            .or_else(|| existing.session_id.clone());

        let mut recent_tools = existing.recent_tools.clone();

        // Merge active tools
        let active_tools =
            if incoming.status == AgentStatus::Running {
                match event.as_str() {
                    "PreToolUse" => {
                        let mut tools = existing.active_tools.clone();
                        for tool in &incoming.active_tools {
                            if tool.name != "Agent" {
                                tools.push(tool.clone());
                            }
                        }
                        tools
                    }
                    "PostToolUse" | "PostToolUseFailure" => {
                        let mut tools = existing.active_tools.clone();
                        if let Some(removing) = incoming.active_tools.first() {
                            if removing.name != "Agent" {
                                // Match by ToolKey first (exact)
                                let pos = tools.iter().position(|t| t.key == removing.key).or_else(
                                    || {
                                        // Fallback: match by name only
                                        tools.iter().position(|t| t.name == removing.name)
                                    },
                                );
                                if let Some(pos) = pos {
                                    let mut completed = tools.remove(pos);
                                    if event == "PostToolUseFailure" {
                                        completed.failure_detail = incoming.failure_detail.clone();
                                    }
                                    push_recent_tool(&mut recent_tools, completed);
                                }
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
        merged.active_tools = active_tools;
        merged.recent_tools = recent_tools;

        if is_silent {
            // Silent: preserve visual state, only update tools
            merged.event_emoji = existing.event_emoji.clone();
            merged.hook_event_name = existing.hook_event_name.clone();
            merged.tool_name = existing.tool_name.clone();
            merged.tool_detail = existing.tool_detail.clone();
            merged.status = existing.status;
            merged.failure_detail = existing.failure_detail.clone();
        } else if event != "PostToolUseFailure" {
            merged.failure_detail = None;
        }

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ToolKey;

    fn make_state(event: &str, status: AgentStatus) -> ClaudeState {
        ClaudeState {
            session_id: None,
            agent_id: None,
            status,
            hook_event_name: event.to_string(),
            event_emoji: "🔧".to_string(),
            tool_name: None,
            tool_detail: None,
            active_tools: Vec::new(),
            recent_tools: Vec::new(),
            failure_detail: None,
        }
    }

    fn tool_with_id(name: &str, tool_use_id: &str) -> ActiveTool {
        ActiveTool {
            key: ToolKey::Claude {
                tool_use_id: tool_use_id.to_string(),
            },
            name: name.to_string(),
            detail: None,
            failure_detail: None,
        }
    }

    #[test]
    fn test_pre_tool_use_adds_tool() {
        let existing = make_state("PreToolUse", AgentStatus::Running);
        let mut incoming = make_state("PreToolUse", AgentStatus::Running);
        incoming.active_tools.push(tool_with_id("Bash", "toolu_01"));

        let merged = ClaudeState::merge(incoming, &existing);
        assert_eq!(merged.active_tools.len(), 1);
        assert_eq!(merged.active_tools[0].name, "Bash");
    }

    #[test]
    fn test_post_tool_use_removes_by_tool_use_id() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running);
        existing.active_tools.push(tool_with_id("Bash", "toolu_01"));
        existing.active_tools.push(tool_with_id("Read", "toolu_02"));

        let mut incoming = make_state("PostToolUse", AgentStatus::Running);
        incoming.active_tools.push(tool_with_id("Bash", "toolu_01"));

        let merged = ClaudeState::merge(incoming, &existing);
        assert_eq!(merged.active_tools.len(), 1);
        assert_eq!(merged.active_tools[0].name, "Read");
        assert_eq!(merged.recent_tools.len(), 1);
        assert_eq!(merged.recent_tools[0].name, "Bash");
    }

    #[test]
    fn test_post_tool_use_silent_preserves_visual_state() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running);
        existing.event_emoji = "🚀".to_string();
        existing.tool_name = Some("Bash".to_string());
        existing.tool_detail = Some("cargo test".to_string());
        existing.active_tools.push(tool_with_id("Bash", "toolu_01"));

        let mut incoming = make_state("PostToolUse", AgentStatus::Running);
        incoming.active_tools.push(tool_with_id("Bash", "toolu_01"));

        let merged = ClaudeState::merge(incoming, &existing);
        // Silent: visual state preserved from existing
        assert_eq!(merged.event_emoji, "🚀");
        assert_eq!(merged.hook_event_name, "PreToolUse");
        assert_eq!(merged.tool_name, Some("Bash".to_string()));
        assert_eq!(merged.tool_detail, Some("cargo test".to_string()));
        // But tool is removed from active list
        assert!(merged.active_tools.is_empty());
        assert_eq!(merged.recent_tools.len(), 1);
        assert_eq!(merged.recent_tools[0].name, "Bash");
    }

    #[test]
    fn test_post_tool_use_failure_sets_failure_detail() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running);
        existing.active_tools.push(tool_with_id("Bash", "toolu_01"));

        let mut incoming = make_state("PostToolUseFailure", AgentStatus::Running);
        incoming.active_tools.push(tool_with_id("Bash", "toolu_01"));
        incoming.failure_detail = Some("command not found".to_string());

        let merged = ClaudeState::merge(incoming, &existing);
        assert_eq!(merged.failure_detail, Some("command not found".to_string()));
        assert!(merged.active_tools.is_empty());
        assert_eq!(merged.recent_tools.len(), 1);
        assert_eq!(
            merged.recent_tools[0].failure_detail.as_deref(),
            Some("command not found")
        );
    }

    #[test]
    fn test_session_id_preserved_from_existing() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running);
        existing.session_id = Some("session-abc".to_string());

        let incoming = make_state("PreToolUse", AgentStatus::Running);

        let merged = ClaudeState::merge(incoming, &existing);
        assert_eq!(merged.session_id, Some("session-abc".to_string()));
    }

    #[test]
    fn test_session_id_incoming_overrides() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running);
        existing.session_id = Some("old-session".to_string());

        let mut incoming = make_state("PreToolUse", AgentStatus::Running);
        incoming.session_id = Some("new-session".to_string());

        let merged = ClaudeState::merge(incoming, &existing);
        assert_eq!(merged.session_id, Some("new-session".to_string()));
    }

    #[test]
    fn test_non_running_clears_active_tools() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running);
        existing.active_tools.push(tool_with_id("Bash", "toolu_01"));

        let incoming = make_state("Stop", AgentStatus::Waiting);

        let merged = ClaudeState::merge(incoming, &existing);
        assert!(merged.active_tools.is_empty());
        // recent_tools preserved across Stop/Waiting transitions to avoid visual flicker
        assert_eq!(merged.recent_tools.len(), 1);
        assert_eq!(merged.recent_tools[0].name, "Bash");
    }

    #[test]
    fn test_agent_tool_not_added_to_active() {
        let existing = make_state("PreToolUse", AgentStatus::Running);
        let mut incoming = make_state("PreToolUse", AgentStatus::Running);
        incoming.active_tools.push(ActiveTool {
            key: ToolKey::Claude {
                tool_use_id: "toolu_99".to_string(),
            },
            name: "Agent".to_string(),
            detail: None,
            failure_detail: None,
        });

        let merged = ClaudeState::merge(incoming, &existing);
        assert!(merged.active_tools.is_empty());
    }

    #[test]
    fn test_failure_detail_cleared_on_non_failure_event() {
        let mut existing = make_state("PostToolUseFailure", AgentStatus::Running);
        existing.failure_detail = Some("old error".to_string());

        let incoming = make_state("PreToolUse", AgentStatus::Running);

        let merged = ClaudeState::merge(incoming, &existing);
        assert!(merged.failure_detail.is_none());
    }

    #[test]
    fn test_post_tool_use_fallback_name_match() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running);
        // Existing tool with a different tool_use_id
        existing.active_tools.push(ActiveTool {
            key: ToolKey::Claude {
                tool_use_id: "toolu_old".to_string(),
            },
            name: "Bash".to_string(),
            detail: None,
            failure_detail: None,
        });

        // Incoming removes by name only (different tool_use_id)
        let mut incoming = make_state("PostToolUse", AgentStatus::Running);
        incoming.active_tools.push(ActiveTool {
            key: ToolKey::Claude {
                tool_use_id: "toolu_new".to_string(),
            },
            name: "Bash".to_string(),
            detail: None,
            failure_detail: None,
        });

        let merged = ClaudeState::merge(incoming, &existing);
        // Fallback name match should remove the tool
        assert!(merged.active_tools.is_empty());
    }
}
