use super::state::AgentSource;

pub fn detect_agent_source(
    command: &str,
    window_name: Option<&str>,
    pane_title: Option<&str>,
) -> Option<AgentSource> {
    if is_claude_command(command) {
        return Some(AgentSource::Claude);
    }
    if is_codex_command(command, window_name, pane_title) {
        return Some(AgentSource::Codex);
    }
    if is_opencode_command(command, window_name, pane_title) {
        return Some(AgentSource::OpenCode);
    }
    None
}

pub fn is_claude_command(command: &str) -> bool {
    command == "claude"
}

pub fn is_codex_command(
    command: &str,
    window_name: Option<&str>,
    pane_title: Option<&str>,
) -> bool {
    command == "codex"
        || command.starts_with("codex-")
        || window_name.is_some_and(|name| name.to_lowercase().contains("codex"))
        || pane_title.is_some_and(|title| title.to_lowercase().contains("codex"))
}

/// Check if a pane still looks like it hosts OpenCode, based on pane_title and window_name.
/// This is more relaxed than `is_opencode_command` — it doesn't require `node` as the
/// current command, because OpenCode tools like bash temporarily change the pane's command.
/// Use this to verify a cached agent state is still valid.
pub fn is_opencode_pane(window_name: Option<&str>, pane_title: Option<&str>) -> bool {
    window_name.is_some_and(|name| name.to_lowercase().contains("opencode"))
        || pane_title.is_some_and(|title| {
            let lower = title.to_lowercase();
            lower.contains("opencode") || lower.starts_with("oc ")
        })
}

fn is_opencode_command(command: &str, window_name: Option<&str>, pane_title: Option<&str>) -> bool {
    command == "node"
        && (window_name.is_some_and(|name| name.to_lowercase().contains("opencode"))
            || pane_title.is_some_and(|title| {
                let lower = title.to_lowercase();
                lower.contains("opencode") || lower.starts_with("oc ")
            }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_agent_source_codex_commands() {
        assert_eq!(
            detect_agent_source("codex", None, None),
            Some(AgentSource::Codex)
        );
        assert_eq!(
            detect_agent_source("codex-aarch64-apple-darwin", None, None),
            Some(AgentSource::Codex)
        );
        assert_eq!(
            detect_agent_source("codex-x86_64-unknown-linux-gnu", None, None),
            Some(AgentSource::Codex)
        );
        assert_eq!(
            detect_agent_source("codex-aarch64-a", None, None),
            Some(AgentSource::Codex)
        );
        assert_eq!(
            detect_agent_source("bash", Some("codex"), None),
            Some(AgentSource::Codex)
        );
        assert_eq!(
            detect_agent_source("zsh", Some("project"), Some("Codex")),
            Some(AgentSource::Codex)
        );
    }

    #[test]
    fn test_detect_agent_source_rejects_unrelated_codex_prefix() {
        assert_eq!(detect_agent_source("my-codex", None, None), None);
    }

    #[test]
    fn test_detect_agent_source_claude_and_opencode() {
        assert_eq!(
            detect_agent_source("claude", Some("anything"), Some("anything")),
            Some(AgentSource::Claude)
        );
        assert_eq!(
            detect_agent_source("node", Some("OpenCode"), None),
            Some(AgentSource::OpenCode)
        );
        assert_eq!(
            detect_agent_source("node", Some("project"), Some("OpenCode pane")),
            Some(AgentSource::OpenCode)
        );
        assert_eq!(
            detect_agent_source("node", Some("project"), Some("OC | Greeting")),
            Some(AgentSource::OpenCode)
        );
        assert_eq!(
            detect_agent_source("node", Some("project"), Some("bash")),
            None
        );
    }
}
