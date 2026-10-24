use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::agent::state::{AgentState, AgentStatus};
use crate::agent::{SubagentInfo, SubagentStatus};
use crate::git::PrInfo;
use crate::usage::Usage;

/// Directory for all persistent state files.
pub fn state_dir() -> PathBuf {
    crate::ipc::socket_dir()
}

/// Path to the agent states JSONL file.
pub fn agent_states_path() -> PathBuf {
    state_dir().join("agent_states.jsonl")
}

/// Path to the git cache JSON file.
pub fn git_cache_path() -> PathBuf {
    state_dir().join("git_cache.json")
}

/// Path to the usage JSON file.
pub fn usage_path() -> PathBuf {
    state_dir().join("usage.json")
}

/// Append an AgentState as a single JSON line to the JSONL file.
/// Creates the file and parent directory if they don't exist.
pub fn append_agent_state(state: &AgentState) -> anyhow::Result<()> {
    let path = agent_states_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let mut json = serde_json::to_string(state)?;
    json.push('\n');
    file.write_all(json.as_bytes())?;
    Ok(())
}

/// Reconstruct the current agent states from a JSONL event log.
/// Reads all lines, applies them in order using the same merge logic as the TUI:
/// - `Ended` → remove
/// - Otherwise → insert (last write wins for the same tmux_pane)
///
/// Also filters out states that are likely stale (older than 24 hours),
/// since the TUI that wrote them may have been gone for a long time.
pub fn load_agent_states_from(path: &Path) -> HashMap<String, AgentState> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let stale_threshold = 24 * 60 * 60; // 24 hours

    let mut states: HashMap<String, AgentState> = HashMap::new();
    let reader = std::io::BufReader::new(file);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let state: AgentState = match serde_json::from_str(line) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Skip stale entries (older than 24 hours)
        if now.saturating_sub(state.updated_at) > stale_threshold {
            continue;
        }

        if state.status() == AgentStatus::Ended {
            states.remove(&state.tmux_pane);
        } else {
            // Last write wins — but use per-agent merge if same source
            if let Some(existing) = states.get(&state.tmux_pane) {
                let merged = state.merge_with(existing);
                states.insert(merged.tmux_pane.clone(), merged);
            } else {
                states.insert(state.tmux_pane.clone(), state);
            }
        }
    }

    states
}

/// Load agent states from the default path.
pub fn load_agent_states() -> HashMap<String, AgentState> {
    load_agent_states_from(&agent_states_path())
}

/// Compact the agent states JSONL file by rewriting it with only the current
/// live states. Call this periodically (e.g., on TUI startup) to prevent
/// unbounded file growth.
#[allow(dead_code)]
pub fn compact_agent_states_to(
    states: &HashMap<String, AgentState>,
    path: &Path,
) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    for state in states.values() {
        let mut json = serde_json::to_string(state)?;
        json.push('\n');
        file.write_all(json.as_bytes())?;
    }
    Ok(())
}

/// Compact agent states at the default path.
#[allow(dead_code)]
pub fn compact_agent_states(states: &HashMap<String, AgentState>) -> anyhow::Result<()> {
    compact_agent_states_to(states, &agent_states_path())
}

// ---- Subagent persistence ----

/// Path to the subagent states JSONL file.
pub fn subagent_states_path() -> PathBuf {
    state_dir().join("subagent_states.jsonl")
}

/// Serializable subagent state entry with pane_id.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SubagentStateEntry {
    pane_id: String,
    subagent: SubagentInfo,
}

/// Append a subagent state to the JSONL log.
pub fn append_subagent_state(pane_id: &str, subagent: &SubagentInfo) -> anyhow::Result<()> {
    let path = subagent_states_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let entry = SubagentStateEntry {
        pane_id: pane_id.to_string(),
        subagent: subagent.clone(),
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let mut json = serde_json::to_string(&entry)?;
    json.push('\n');
    file.write_all(json.as_bytes())?;
    Ok(())
}

/// Load subagent states from JSONL, returning the last state per (pane, agent_id).
pub fn load_subagent_states() -> HashMap<(String, String), SubagentInfo> {
    let path = subagent_states_path();
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };

    let reader = std::io::BufReader::new(file);
    let mut states: HashMap<(String, String), SubagentInfo> = HashMap::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: SubagentStateEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let key = (entry.pane_id, entry.subagent.id.clone());
        let ts = entry.subagent.updated_at;

        match states.get(&key) {
            Some(existing) if existing.updated_at >= ts => {}
            _ => {
                states.insert(key, entry.subagent);
            }
        }
    }

    // Remove ended subagents
    states.retain(|_, info| info.state != SubagentStatus::Ended);
    states
}

/// Load completed subagent counts from the JSONL log.
/// Deduplicates by (pane_id, agent_id) to avoid counting the same subagent multiple times.
pub fn load_completed_counts() -> HashMap<String, u32> {
    let path = subagent_states_path();
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };

    let reader = std::io::BufReader::new(file);
    let mut counts: HashMap<String, u32> = HashMap::new();
    // Track which (pane_id, agent_id) pairs have already been counted
    let mut counted: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: SubagentStateEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.subagent.state == SubagentStatus::Ended {
            let key = (entry.pane_id.clone(), entry.subagent.id.clone());
            if !counted.contains(&key) {
                counted.insert(key);
                *counts.entry(entry.pane_id).or_insert(0) += 1;
            }
        }
    }

    counts
}

// ---- Git cache persistence ----

/// Serializable git cache entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitCacheEntry {
    pub path: String,
    pub branch: Option<String>,
    pub pr: Option<PrInfo>,
    pub repo_name: Option<String>,
    pub toplevel: Option<String>,
    pub worktree_name: Option<String>,
}

/// Serializable wrapper for the entire git cache.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GitCacheSnapshot {
    pub entries: Vec<GitCacheEntry>,
}

/// Save git cache snapshot to disk.
pub fn save_git_cache(entries: &[GitCacheEntry]) -> anyhow::Result<()> {
    let path = git_cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let snapshot = GitCacheSnapshot {
        entries: entries.to_vec(),
    };
    let json = serde_json::to_string_pretty(&snapshot)?;
    // Atomic write: write to temp file then rename
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Load git cache snapshot from disk.
pub fn load_git_cache() -> Option<Vec<GitCacheEntry>> {
    let path = git_cache_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let snapshot: GitCacheSnapshot = serde_json::from_str(&data).ok()?;
    Some(snapshot.entries)
}

// ---- Usage persistence ----

/// Serializable usage data with timestamp.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct UsageSnapshot {
    pub five_hour: f64,
    pub seven_day: f64,
    /// When this snapshot was saved (unix timestamp).
    pub saved_at: u64,
}

/// Save usage data to disk.
pub fn save_usage(usage: &Usage) -> anyhow::Result<()> {
    let path = usage_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let snapshot = UsageSnapshot {
        five_hour: usage.five_hour,
        seven_day: usage.seven_day,
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let json = serde_json::to_string_pretty(&snapshot)?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Load usage data from disk.
pub fn load_usage() -> Option<Usage> {
    let path = usage_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let snapshot: UsageSnapshot = serde_json::from_str(&data).ok()?;
    Some(Usage {
        five_hour: snapshot.five_hour,
        seven_day: snapshot.seven_day,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::claude::ClaudeState;
    use crate::agent::state::AgentData;

    fn make_claude_state(pane_id: &str, status: AgentStatus) -> AgentState {
        AgentState::new(
            pane_id.to_string(),
            AgentData::Claude(ClaudeState {
                session_id: None,
                agent_id: None,
                status,
                hook_event_name: "PreToolUse".to_string(),
                event_emoji: "🔧".to_string(),
                tool_name: None,
                tool_detail: None,
                active_tools: Vec::new(),
                recent_tools: Vec::new(),
                failure_detail: None,
            }),
        )
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("chikuwa_persist_test");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn write_jsonl(path: &Path, lines: &[&str]) {
        let mut f = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
    }

    #[test]
    fn test_append_and_load_agent_states() {
        let dir = temp_dir();
        let path = dir.join("test_append.jsonl");
        let _ = std::fs::remove_file(&path);

        let state1 = make_claude_state("%0", AgentStatus::Running);
        let state2 = make_claude_state("%1", AgentStatus::Waiting);

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&state1).unwrap(),
                &serde_json::to_string(&state2).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        assert_eq!(states.len(), 2);
        assert!(states.contains_key("%0"));
        assert!(states.contains_key("%1"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_agent_states_ended_removes() {
        let dir = temp_dir();
        let path = dir.join("test_ended.jsonl");
        let _ = std::fs::remove_file(&path);

        let running = make_claude_state("%0", AgentStatus::Running);
        let ended_state = ClaudeState {
            session_id: None,
            agent_id: None,
            status: AgentStatus::Ended,
            hook_event_name: "SessionEnd".to_string(),
            event_emoji: "🏁".to_string(),
            tool_name: None,
            tool_detail: None,
            active_tools: Vec::new(),
            recent_tools: Vec::new(),
            failure_detail: None,
        };
        let ended = AgentState::new("%0".to_string(), AgentData::Claude(ended_state.clone()));

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&running).unwrap(),
                &serde_json::to_string(&ended).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        assert!(!states.contains_key("%0"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_agent_states_last_write_wins() {
        let dir = temp_dir();
        let path = dir.join("test_merge.jsonl");
        let _ = std::fs::remove_file(&path);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut state1 = make_claude_state("%0", AgentStatus::Running);
        // Set session_id on first state
        if let AgentData::Claude(ref mut c) = state1.data {
            c.session_id = Some("old-session".to_string());
        }
        state1.updated_at = now - 10;

        let mut state2 = make_claude_state("%0", AgentStatus::Waiting);
        // No session_id on second state - should be preserved from first
        state2.updated_at = now;

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&state1).unwrap(),
                &serde_json::to_string(&state2).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        let result = states.get("%0").unwrap();
        assert_eq!(result.status(), AgentStatus::Waiting);
        // session_id preserved from earlier entry (ClaudeState::merge preserves it)
        assert_eq!(result.session_id(), Some("old-session"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_agent_states_stale_filtered() {
        let dir = temp_dir();
        let path = dir.join("test_stale.jsonl");
        let _ = std::fs::remove_file(&path);

        let fresh = make_claude_state("%0", AgentStatus::Running);
        let mut stale = make_claude_state("%1", AgentStatus::Running);
        stale.updated_at = stale.updated_at.saturating_sub(25 * 60 * 60); // 25 hours ago

        write_jsonl(
            &path,
            &[
                &serde_json::to_string(&fresh).unwrap(),
                &serde_json::to_string(&stale).unwrap(),
            ],
        );

        let states = load_agent_states_from(&path);
        assert!(states.contains_key("%0"));
        assert!(!states.contains_key("%1")); // stale filtered out

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_git_cache_roundtrip() {
        let entries = vec![GitCacheEntry {
            path: "/home/user/project".to_string(),
            branch: Some("main".to_string()),
            pr: Some(PrInfo {
                number: 42,
                title: "Fix bug".to_string(),
            }),
            repo_name: Some("owner/repo".to_string()),
            toplevel: Some("/home/user/project".to_string()),
            worktree_name: None,
        }];
        let json = serde_json::to_string_pretty(&GitCacheSnapshot {
            entries: entries.clone(),
        })
        .unwrap();
        let parsed: GitCacheSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, "/home/user/project");
        assert_eq!(parsed.entries[0].branch, Some("main".to_string()));
    }

    #[test]
    fn test_usage_roundtrip() {
        let usage = Usage {
            five_hour: 0.63,
            seven_day: 0.19,
        };
        let json = serde_json::to_string_pretty(&UsageSnapshot {
            five_hour: usage.five_hour,
            seven_day: usage.seven_day,
            saved_at: 1234567890,
        })
        .unwrap();
        let parsed: UsageSnapshot = serde_json::from_str(&json).unwrap();
        assert!((parsed.five_hour - 0.63).abs() < f64::EPSILON);
        assert!((parsed.seven_day - 0.19).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compact_agent_states() {
        let dir = temp_dir();
        let path = dir.join("test_compact.jsonl");
        let _ = std::fs::remove_file(&path);

        let mut states = HashMap::new();
        let mut state = make_claude_state("%0", AgentStatus::Running);
        if let AgentData::Claude(ref mut c) = state.data {
            c.session_id = Some("s1".to_string());
        }
        states.insert("%0".to_string(), state);

        compact_agent_states_to(&states, &path).unwrap();

        // Read back and verify
        let data = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = data.trim().lines().collect();
        assert_eq!(lines.len(), 1);
        let parsed: AgentState = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.tmux_pane, "%0");

        let _ = std::fs::remove_file(&path);
    }
}
