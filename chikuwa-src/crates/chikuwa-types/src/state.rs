use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Unique identifier for an in-flight tool call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolKey {
    /// Claude Code: exact match via tool_use_id
    Claude { tool_use_id: String },
    /// Codex CLI: exact match via tool_use_id
    Codex { tool_use_id: String },
    /// OpenCode: no unique ID, approximate match via name+detail
    #[serde(rename = "opencode")]
    OpenCode {
        name: String,
        detail: Option<String>,
    },
}

/// A tool call that is currently in progress.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActiveTool {
    pub key: ToolKey,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
}

pub const RECENT_TOOLS_LIMIT: usize = 3;

pub fn push_recent_tool(recent_tools: &mut Vec<ActiveTool>, tool: ActiveTool) {
    if let Some(pos) = recent_tools
        .iter()
        .position(|existing| existing.key == tool.key)
    {
        recent_tools.remove(pos);
    }
    recent_tools.push(tool);
    let overflow = recent_tools.len().saturating_sub(RECENT_TOOLS_LIMIT);
    if overflow > 0 {
        recent_tools.drain(0..overflow);
    }
}

/// Which agent produced this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    Claude,
    OpenCode,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Started,
    Running,
    Waiting,
    Permission,
    Ended,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Started => write!(f, "started"),
            AgentStatus::Running => write!(f, "running"),
            AgentStatus::Waiting => write!(f, "waiting"),
            AgentStatus::Permission => write!(f, "permission"),
            AgentStatus::Ended => write!(f, "ended"),
        }
    }
}

/// Read-only view of agent state for UI rendering.
#[allow(dead_code)]
pub trait AgentView {
    fn status(&self) -> AgentStatus;
    fn source(&self) -> AgentSource;
    fn session_id(&self) -> Option<&str>;
    fn agent_id(&self) -> Option<&str>;
    fn event_label(&self) -> &str;
    fn event_emoji(&self) -> Option<&str>;
    fn recent_tools(&self) -> &[ActiveTool];
    fn active_tools(&self) -> &[ActiveTool];
    fn current_tool_name(&self) -> Option<&str>;
    fn current_tool_detail(&self) -> Option<&str>;
    fn failure_detail(&self) -> Option<&str>;
}

/// Per-agent state data, tagged by source.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentData {
    Claude(super::claude::ClaudeState),
    #[serde(rename = "opencode")]
    OpenCode(super::opencode_state::OpenCodeState),
    Codex(super::codex_state::CodexState),
}

/// Top-level agent state tracked by the TUI.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentState {
    pub tmux_pane: String,
    pub updated_at: u64,
    pub data: AgentData,
}

impl AgentData {
    /// Merge incoming data with existing data, returning the merged result.
    /// If the agent types differ, returns the incoming data unchanged.
    pub fn merge(incoming: AgentData, existing: &AgentData) -> AgentData {
        match (&incoming, existing) {
            (AgentData::Claude(in_c), AgentData::Claude(ex_c)) => {
                AgentData::Claude(super::claude::ClaudeState::merge(in_c.clone(), ex_c))
            }
            (AgentData::OpenCode(in_o), AgentData::OpenCode(ex_o)) => AgentData::OpenCode(
                super::opencode_state::OpenCodeState::merge(in_o.clone(), ex_o),
            ),
            (AgentData::Codex(in_c), AgentData::Codex(ex_c)) => {
                AgentData::Codex(super::codex_state::CodexState::merge(in_c.clone(), ex_c))
            }
            _ => incoming,
        }
    }
}

impl AgentState {
    pub fn new(tmux_pane: String, data: AgentData) -> Self {
        Self {
            tmux_pane,
            updated_at: now(),
            data,
        }
    }

    /// Merge this state with an existing state for the same tmux_pane.
    /// Returns a new AgentState with merged data.
    pub fn merge_with(mut self, existing: &AgentState) -> AgentState {
        self.data = AgentData::merge(self.data, &existing.data);
        self
    }

    pub fn status(&self) -> AgentStatus {
        match &self.data {
            AgentData::Claude(c) => c.status,
            AgentData::OpenCode(o) => o.status,
            AgentData::Codex(c) => c.status,
        }
    }

    #[allow(dead_code)]
    pub fn session_id(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.session_id.as_deref(),
            AgentData::OpenCode(o) => o.session_id.as_deref(),
            AgentData::Codex(c) => c.session_id.as_deref(),
        }
    }

    pub fn agent_id(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.agent_id.as_deref(),
            AgentData::OpenCode(_) => None,
            AgentData::Codex(c) => c.agent_id.as_deref(),
        }
    }

    pub fn source(&self) -> AgentSource {
        match &self.data {
            AgentData::Claude(_) => AgentSource::Claude,
            AgentData::OpenCode(_) => AgentSource::OpenCode,
            AgentData::Codex(_) => AgentSource::Codex,
        }
    }
}

impl AgentView for AgentState {
    fn status(&self) -> AgentStatus {
        self.status()
    }
    fn source(&self) -> AgentSource {
        self.source()
    }
    fn session_id(&self) -> Option<&str> {
        self.session_id()
    }
    fn agent_id(&self) -> Option<&str> {
        self.agent_id()
    }
    fn event_label(&self) -> &str {
        match &self.data {
            AgentData::Claude(c) => &c.hook_event_name,
            AgentData::OpenCode(o) => o.event_type.as_deref().unwrap_or("Agent"),
            AgentData::Codex(c) => &c.hook_event_name,
        }
    }
    fn event_emoji(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => Some(&c.event_emoji),
            AgentData::OpenCode(o) => o.event_emoji.as_deref(),
            AgentData::Codex(c) => Some(&c.event_emoji),
        }
    }
    fn active_tools(&self) -> &[ActiveTool] {
        match &self.data {
            AgentData::Claude(c) => &c.active_tools,
            AgentData::OpenCode(o) => &o.active_tools,
            AgentData::Codex(c) => &c.active_tools,
        }
    }
    fn recent_tools(&self) -> &[ActiveTool] {
        match &self.data {
            AgentData::Claude(c) => &c.recent_tools,
            AgentData::OpenCode(o) => &o.recent_tools,
            AgentData::Codex(c) => &c.recent_tools,
        }
    }
    fn current_tool_name(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.tool_name.as_deref(),
            AgentData::OpenCode(o) => o.tool_name.as_deref(),
            AgentData::Codex(c) => c.tool_name.as_deref(),
        }
    }
    fn current_tool_detail(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.tool_detail.as_deref(),
            AgentData::OpenCode(o) => o.tool_detail.as_deref(),
            AgentData::Codex(c) => c.tool_detail.as_deref(),
        }
    }
    fn failure_detail(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.failure_detail.as_deref(),
            AgentData::OpenCode(_) => None,
            AgentData::Codex(c) => c.failure_detail.as_deref(),
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::ClaudeState;

    #[test]
    fn test_agent_status_serialize() {
        assert_eq!(
            serde_json::to_string(&AgentStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&AgentStatus::Waiting).unwrap(),
            "\"waiting\""
        );
    }

    #[test]
    fn test_agent_status_deserialize() {
        assert_eq!(
            serde_json::from_str::<AgentStatus>("\"running\"").unwrap(),
            AgentStatus::Running
        );
        assert_eq!(
            serde_json::from_str::<AgentStatus>("\"started\"").unwrap(),
            AgentStatus::Started
        );
    }

    #[test]
    fn test_agent_status_display() {
        assert_eq!(AgentStatus::Running.to_string(), "running");
        assert_eq!(AgentStatus::Waiting.to_string(), "waiting");
        assert_eq!(AgentStatus::Permission.to_string(), "permission");
        assert_eq!(AgentStatus::Started.to_string(), "started");
        assert_eq!(AgentStatus::Ended.to_string(), "ended");
    }

    #[test]
    fn test_agent_state_new() {
        let state = AgentState::new(
            "%5".to_string(),
            AgentData::Claude(ClaudeState {
                session_id: None,
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
        );
        assert_eq!(state.tmux_pane, "%5");
        assert_eq!(state.status(), AgentStatus::Running);
        assert!(state.session_id().is_none());
        assert!(state.updated_at > 0);
    }

    #[test]
    fn test_agent_state_roundtrip_json() {
        let state = AgentState::new(
            "%5".to_string(),
            AgentData::Claude(ClaudeState {
                session_id: Some("abc123".to_string()),
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
        );
        // Override updated_at for deterministic test
        let mut state = state;
        state.updated_at = 1234567890;

        let json = serde_json::to_string(&state).unwrap();
        let parsed: AgentState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tmux_pane, "%5");
        assert_eq!(parsed.session_id(), Some("abc123"));
        assert_eq!(parsed.status(), AgentStatus::Running);
        assert_eq!(parsed.updated_at, 1234567890);
    }

    #[test]
    fn test_agent_state_deserialize_minimal() {
        let json = r#"{"tmux_pane":"%0","updated_at":100,"data":{"type":"claude","status":"running","hook_event_name":"PreToolUse","event_emoji":"🔧"}}"#;
        let state: AgentState = serde_json::from_str(json).unwrap();
        assert_eq!(state.tmux_pane, "%0");
        assert_eq!(state.status(), AgentStatus::Running);
        assert!(state.session_id().is_none());
    }

    #[test]
    fn test_codex_agent_state_roundtrip_json() {
        let state = AgentState::new(
            "%1".to_string(),
            AgentData::Codex(crate::codex_state::CodexState::new(
                "SessionStart",
                AgentStatus::Started,
                "🚀",
            )),
        );

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: AgentState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.tmux_pane, "%1");
        assert_eq!(deserialized.status(), AgentStatus::Started);
        assert_eq!(deserialized.source(), AgentSource::Codex);
        assert_eq!(deserialized.event_label(), "SessionStart");
        assert_eq!(deserialized.event_emoji(), Some("🚀"));
    }

    #[test]
    fn test_tool_key_equality() {
        let k1 = ToolKey::Claude {
            tool_use_id: "toolu_01".to_string(),
        };
        let k2 = ToolKey::Claude {
            tool_use_id: "toolu_01".to_string(),
        };
        let k3 = ToolKey::Claude {
            tool_use_id: "toolu_02".to_string(),
        };
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    fn test_tool(name: &str, id: &str) -> ActiveTool {
        ActiveTool {
            key: ToolKey::Claude {
                tool_use_id: id.to_string(),
            },
            name: name.to_string(),
            detail: Some(format!("{name} detail")),
            failure_detail: None,
        }
    }

    #[test]
    fn test_push_recent_tool_keeps_limit_and_oldest_to_newest_order() {
        let mut recent = Vec::new();

        push_recent_tool(&mut recent, test_tool("Bash", "1"));
        push_recent_tool(&mut recent, test_tool("Read", "2"));
        push_recent_tool(&mut recent, test_tool("Edit", "3"));
        push_recent_tool(&mut recent, test_tool("Write", "4"));

        assert_eq!(RECENT_TOOLS_LIMIT, 3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].name, "Read");
        assert_eq!(recent[1].name, "Edit");
        assert_eq!(recent[2].name, "Write");
    }

    #[test]
    fn test_push_recent_tool_dedupes_by_key_and_moves_to_newest() {
        let mut recent = Vec::new();

        push_recent_tool(&mut recent, test_tool("Bash", "1"));
        push_recent_tool(&mut recent, test_tool("Read", "2"));
        push_recent_tool(&mut recent, test_tool("Bash", "1"));

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].name, "Read");
        assert_eq!(recent[1].name, "Bash");
    }

    #[test]
    fn test_codex_tool_key_serializes() {
        let key = ToolKey::Codex {
            tool_use_id: "call-123".to_string(),
        };

        let json = serde_json::to_string(&key).unwrap();

        assert_eq!(json, r#"{"type":"codex","tool_use_id":"call-123"}"#);
    }
}
