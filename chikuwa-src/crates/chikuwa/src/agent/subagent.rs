use serde::{Deserialize, Serialize};

use super::state::{ActiveTool, AgentStatus};

/// Status specific to subagents (extends AgentStatus with lifecycle info)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubagentStatus {
    Running,
    Waiting,
    Ended,
}

impl From<AgentStatus> for SubagentStatus {
    fn from(status: AgentStatus) -> Self {
        match status {
            AgentStatus::Started | AgentStatus::Running => SubagentStatus::Running,
            AgentStatus::Waiting | AgentStatus::Permission => SubagentStatus::Waiting,
            AgentStatus::Ended => SubagentStatus::Ended,
        }
    }
}

/// Information about a subagent spawned by the main agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    /// Full agent_id from Claude Code (e.g., "a35b03b884b2cb68f")
    pub id: String,
    /// Last 8 characters of id for display (e.g., "a35b03b8")
    pub short_id: String,
    /// Description from the Task tool that spawned this subagent
    pub description: Option<String>,
    /// Current status
    pub state: SubagentStatus,
    /// Currently active tools
    #[serde(default)]
    pub tools: Vec<ActiveTool>,
    /// Recently completed tools, oldest to newest
    #[serde(default)]
    pub recent_tools: Vec<ActiveTool>,
    /// Unix timestamp of last update
    pub updated_at: u64,
}

impl SubagentInfo {
    pub fn new(id: String, description: Option<String>) -> Self {
        let short_id = if id.len() > 8 {
            id[id.len() - 8..].to_string()
        } else {
            id.clone()
        };
        Self {
            id,
            short_id,
            description,
            state: SubagentStatus::Running,
            tools: Vec::new(),
            recent_tools: Vec::new(),
            updated_at: now(),
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

    #[test]
    fn test_subagent_info_new() {
        let info = SubagentInfo::new(
            "a35b03b884b2cb68f".to_string(),
            Some("Search codebase".to_string()),
        );
        assert_eq!(info.id, "a35b03b884b2cb68f");
        assert_eq!(info.short_id, "4b2cb68f"); // Last 8 chars
        assert_eq!(info.description, Some("Search codebase".to_string()));
        assert_eq!(info.state, SubagentStatus::Running);
        assert!(info.tools.is_empty());
        assert!(info.recent_tools.is_empty());
    }

    #[test]
    fn test_subagent_info_short_id_truncation() {
        let info = SubagentInfo::new("short".to_string(), None);
        assert_eq!(info.short_id, "short");
    }

    #[test]
    fn test_subagent_status_from_agent_status() {
        assert_eq!(
            SubagentStatus::from(AgentStatus::Started),
            SubagentStatus::Running
        );
        assert_eq!(
            SubagentStatus::from(AgentStatus::Running),
            SubagentStatus::Running
        );
        assert_eq!(
            SubagentStatus::from(AgentStatus::Waiting),
            SubagentStatus::Waiting
        );
        assert_eq!(
            SubagentStatus::from(AgentStatus::Ended),
            SubagentStatus::Ended
        );
    }
}
