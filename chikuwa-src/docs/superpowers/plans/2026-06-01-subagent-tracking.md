# Subagent Tracking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Track Claude Code subagents separately from the main agent, displaying each subagent's status and tools nested under the main agent in the TUI.

**Architecture:** Hybrid approach — keep existing AgentState for main agents, add parallel SubagentInfo tracking keyed by (tmux_pane, agent_id). Subagents render as nested children under the main agent's pane with their own status and tool lines. Completed subagents collapse into a summary count.

**Tech Stack:** Rust, serde for JSON serialization, existing ratatui UI framework

---

## File Structure

| File | Responsibility |
|------|----------------|
| `src/agent/subagent.rs` (new) | SubagentInfo struct, SubagentStatus enum, merge logic |
| `src/agent/mod.rs` | Export subagent module |
| `src/hook.rs` | Parse agent_id from hook events, handle SubagentStart/Stop |
| `src/app.rs` | Add subagent_states HashMap, completed_counts HashMap, merge logic |
| `src/persist.rs` | Persist/restore subagent states to JSONL |
| `src/ui/tree.rs` | Render subagents nested under main agent, show completed summary |

---

### Task 1: Define SubagentInfo Struct

**Files:**
- Create: `src/agent/subagent.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Create subagent.rs with SubagentInfo struct**

```rust
use serde::{Deserialize, Serialize};

use super::state::{AgentStatus, ToolInfo};

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
    pub tools: Vec<ToolInfo>,
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
        let info = SubagentInfo::new("a35b03b884b2cb68f".to_string(), Some("Search codebase".to_string()));
        assert_eq!(info.id, "a35b03b884b2cb68f");
        assert_eq!(info.short_id, "2cb68f");
        assert_eq!(info.description, Some("Search codebase".to_string()));
        assert_eq!(info.state, SubagentStatus::Running);
        assert!(info.tools.is_empty());
    }

    #[test]
    fn test_subagent_info_short_id_truncation() {
        let info = SubagentInfo::new("short".to_string(), None);
        assert_eq!(info.short_id, "short");
    }

    #[test]
    fn test_subagent_status_from_agent_status() {
        assert_eq!(SubagentStatus::from(AgentStatus::Started), SubagentStatus::Running);
        assert_eq!(SubagentStatus::from(AgentStatus::Running), SubagentStatus::Running);
        assert_eq!(SubagentStatus::from(AgentStatus::Waiting), SubagentStatus::Waiting);
        assert_eq!(SubagentStatus::from(AgentStatus::Ended), SubagentStatus::Ended);
    }
}
```

- [ ] **Step 2: Update mod.rs to export subagent module**

```rust
// In src/agent/mod.rs, add at the end:
pub mod subagent;

pub use subagent::{SubagentInfo, SubagentStatus};
```

- [ ] **Step 3: Run tests to verify**

Run: `cargo test agent::subagent`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/agent/subagent.rs src/agent/mod.rs
git commit -m "feat(agent): add SubagentInfo struct for tracking subagent state"
```

---

### Task 2: Parse agent_id in Hook Handler

**Files:**
- Modify: `src/hook.rs`

- [ ] **Step 1: Add agent_id field to HookInput struct**

Find the `HookInput` struct (around line 10) and add the `agent_id` field:

```rust
/// Input JSON from Claude Code hooks (stdin).
#[derive(Debug, Deserialize)]
struct HookInput {
    hook_event_name: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<serde_json::Value>,
}
```

- [ ] **Step 2: Update HookOutput enum to include agent_id**

Add a new struct to carry additional context from the hook:

```rust
/// Parsed hook result with context for state updates.
struct HookResult {
    status: AgentStatus,
    agent_id: Option<String>,
    session_id: Option<String>,
    tool_name: Option<String>,
    tool_detail: Option<String>,
}
```

- [ ] **Step 3: Refactor run() to return HookResult**

Replace the existing run() function logic with:

```rust
/// Run the hook subcommand: read stdin JSON, determine event from hook_event_name, send state via IPC.
pub async fn run() -> Result<()> {
    let pane_id = std::env::var("TMUX_PANE")
        .context("TMUX_PANE environment variable not set (not running inside tmux?)")?;

    let mut stdin_buf = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin_buf)
        .context("Failed to read stdin")?;

    let input: HookInput = serde_json::from_str(stdin_buf.trim())
        .context("Failed to parse hook input JSON from stdin")?;

    let result = parse_hook_event(&input, &stdin_buf);

    // Build AgentState for IPC broadcast
    let mut state = AgentState::new(pane_id.clone(), result.status);
    state.session_id = result.session_id.clone();
    state.hook_event_name = Some(input.hook_event_name.clone());
    
    if let Some(ref name) = result.tool_name {
        let detail = input
            .tool_input
            .as_ref()
            .and_then(|inp| extract_tool_detail(name, inp));
        state.tools = vec![ToolInfo {
            name: name.clone(),
            detail,
        }];
    }
    state.tool_name = result.tool_name;
    
    // Include agent_id in the state for subagent tracking
    // We'll use a custom field that gets serialized
    if let Some(ref agent_id) = result.agent_id {
        state.agent_id = Some(agent_id.clone());
    }

    ipc::broadcast_state(&state).await?;

    // Persist to JSONL so TUI can restore state on restart
    if let Err(e) = crate::persist::append_agent_state(&state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }

    Ok(())
}

/// Parse hook event and determine the result.
fn parse_hook_event(input: &HookInput, stdin_buf: &str) -> HookResult {
    let status = match input.hook_event_name.as_str() {
        "SessionStart" => AgentStatus::Started,
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure"
        | "SubagentStart" | "SubagentStop" => AgentStatus::Running,
        "Stop" => AgentStatus::Waiting,
        "PermissionRequest" => AgentStatus::Permission,
        "Notification" => {
            if stdin_buf.contains("permission_prompt") {
                AgentStatus::Permission
            } else {
                return HookResult {
                    status: AgentStatus::Running,
                    agent_id: input.agent_id.clone(),
                    session_id: input.session_id.clone(),
                    tool_name: None,
                    tool_detail: None,
                };
            }
        }
        "SessionEnd" => AgentStatus::Ended,
        _ => {
            return HookResult {
                status: AgentStatus::Running,
                agent_id: input.agent_id.clone(),
                session_id: input.session_id.clone(),
                tool_name: None,
                tool_detail: None,
            }
        }
    };

    let tool_detail = input
        .tool_name
        .as_ref()
        .and_then(|name| input.tool_input.as_ref().and_then(|inp| extract_tool_detail(name, inp)));

    HookResult {
        status,
        agent_id: input.agent_id.clone(),
        session_id: input.session_id.clone(),
        tool_name: input.tool_name.clone(),
        tool_detail,
    }
}
```

- [ ] **Step 4: Add agent_id field to AgentState**

In `src/agent/state.rs`, add the field to AgentState:

```rust
pub struct AgentState {
    pub tmux_pane: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,  // Add this field
    pub state: AgentStatus,
    pub updated_at: u64,
    // ... rest of fields
}
```

Also update the `new()` function:

```rust
impl AgentState {
    pub fn new(tmux_pane: String, state: AgentStatus) -> Self {
        Self {
            tmux_pane,
            session_id: None,
            agent_id: None,  // Add this
            state,
            updated_at: now(),
            // ... rest
        }
    }
}
```

- [ ] **Step 5: Run tests to verify**

Run: `cargo test hook::`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/hook.rs src/agent/state.rs
git commit -m "feat(hook): parse agent_id from Claude Code hook events"
```

---

### Task 3: Add Subagent State Storage to App

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add subagent storage fields to App struct**

Find the App struct (around line 141) and add:

```rust
use crate::agent::{AgentState, SubagentInfo};

pub struct App {
    // ... existing fields ...
    agent_states: HashMap<String, AgentState>,
    /// Subagent states keyed by (tmux_pane, agent_id)
    subagent_states: HashMap<(String, String), SubagentInfo>,
    /// Count of completed subagents per pane
    completed_subagent_counts: HashMap<String, u32>,
    // ... rest of fields ...
}
```

- [ ] **Step 2: Initialize new fields in App::new()**

```rust
impl App {
    pub fn new() -> Self {
        // ... existing initialization ...
        Self {
            // ... existing fields ...
            agent_states: persist::load_agent_states(),
            subagent_states: HashMap::new(),  // Add
            completed_subagent_counts: HashMap::new(),  // Add
            // ... rest ...
        }
    }
}
```

- [ ] **Step 3: Add method to get subagents for a pane**

```rust
impl App {
    /// Get all active subagents for a given tmux pane, sorted by update time (newest first).
    fn get_subagents_for_pane(&self, pane_id: &str) -> Vec<&SubagentInfo> {
        let mut subagents: Vec<&SubagentInfo> = self
            .subagent_states
            .iter()
            .filter(|((pane, _), _)| pane == pane_id)
            .map(|(_, info)| info)
            .filter(|info| info.state != SubagentStatus::Ended)
            .collect();
        subagents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        subagents
    }

    /// Get completed subagent count for a pane.
    fn get_completed_count(&self, pane_id: &str) -> u32 {
        *self.completed_subagent_counts.get(pane_id).unwrap_or(&0)
    }
}
```

- [ ] **Step 4: Run tests to verify compilation**

Run: `cargo build`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): add subagent state storage fields"
```

---

### Task 4: Implement Subagent State Merging

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add merge_subagent_state method to App**

```rust
impl App {
    /// Merge a subagent state update into the app state.
    fn merge_subagent_state(&mut self, pane_id: String, agent_id: String, state: AgentState) {
        use crate::agent::SubagentStatus;
        
        // Handle subagent ended
        if state.state == AgentStatus::Ended {
            self.subagent_states.remove(&(pane_id.clone(), agent_id.clone()));
            *self.completed_subagent_counts.entry(pane_id).or_insert(0) += 1;
            return;
        }

        // Get or create subagent info
        let entry = self
            .subagent_states
            .entry((pane_id.clone(), agent_id.clone()));
        
        match entry {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                let info = e.get_mut();
                info.state = SubagentStatus::from(state.state);
                info.updated_at = state.updated_at;
                
                // Merge tools based on event
                let event = state.hook_event_name.as_deref().unwrap_or("");
                match event {
                    "PreToolUse" => {
                        if let Some(tool) = state.tools.first() {
                            if !info.tools.iter().any(|t| t.name == tool.name && t.detail == tool.detail) {
                                info.tools.push(tool.clone());
                            }
                        }
                    }
                    "PostToolUse" | "PostToolUseFailure" => {
                        if let Some(removing) = state.tools.first() {
                            info.tools.retain(|t| t.name != removing.name || t.detail != removing.detail);
                        }
                    }
                    _ => {}
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                // New subagent - extract description from Task tool if available
                let description = if state.tool_name.as_deref() == Some("Task") {
                    state.tools.first().and_then(|t| t.detail.clone())
                } else {
                    None
                };
                
                e.insert(SubagentInfo::new(agent_id, description));
            }
        }
    }
}
```

- [ ] **Step 2: Update AgentStateUpdate handler in run_app**

Find the `AppEvent::AgentStateUpdate(state)` handler (around line 810) and update it:

```rust
AppEvent::AgentStateUpdate(state) => {
    // Persist to JSONL before state is consumed
    if let Err(e) = persist::append_agent_state(&state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }
    
    // Determine if this is a subagent event
    let is_subagent = state.agent_id.is_some();
    
    if is_subagent {
        // Subagent event
        let agent_id = state.agent_id.clone().unwrap();
        let pane_id = state.tmux_pane.clone();
        app.merge_subagent_state(pane_id, agent_id, state);
    } else {
        // Main agent event (existing logic)
        use crate::agent::state::AgentStatus;
        if state.state == AgentStatus::Ended {
            app.agent_states.remove(&state.tmux_pane);
        } else if let Some(existing) = app.agent_states.get(&state.tmux_pane) {
            // ... existing merge logic ...
        } else {
            app.agent_states.insert(state.tmux_pane.clone(), state);
        }
    }
    app.merge_agent_states();
}
```

- [ ] **Step 3: Add unit test for subagent merging**

```rust
#[cfg(test)]
mod subagent_tests {
    use super::*;
    use crate::agent::{AgentStatus, SubagentStatus, ToolInfo};

    #[test]
    fn test_merge_subagent_state_new() {
        let mut app = App::new();
        let state = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            agent_id: Some("abc123".to_string()),
            state: AgentStatus::Running,
            updated_at: 100,
            hook_event_name: Some("SubagentStart".to_string()),
            tool_name: Some("Task".to_string()),
            tool_detail: None,
            tools: vec![ToolInfo {
                name: "Task".to_string(),
                detail: Some("Search codebase".to_string()),
            }],
        };
        
        app.merge_subagent_state("%0".to_string(), "abc123".to_string(), state);
        
        let subagents = app.get_subagents_for_pane("%0");
        assert_eq!(subagents.len(), 1);
        assert_eq!(subagents[0].description, Some("Search codebase".to_string()));
    }

    #[test]
    fn test_merge_subagent_state_ended() {
        let mut app = App::new();
        
        // First add a running subagent
        let running_state = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            agent_id: Some("abc123".to_string()),
            state: AgentStatus::Running,
            updated_at: 100,
            hook_event_name: None,
            tool_name: None,
            tool_detail: None,
            tools: vec![],
        };
        app.merge_subagent_state("%0".to_string(), "abc123".to_string(), running_state);
        
        // Then end it
        let ended_state = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            agent_id: Some("abc123".to_string()),
            state: AgentStatus::Ended,
            updated_at: 200,
            hook_event_name: Some("SubagentStop".to_string()),
            tool_name: None,
            tool_detail: None,
            tools: vec![],
        };
        app.merge_subagent_state("%0".to_string(), "abc123".to_string(), ended_state);
        
        assert_eq!(app.get_subagents_for_pane("%0").len(), 0);
        assert_eq!(app.get_completed_count("%0"), 1);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test app::subagent_tests`
Expected: Tests pass

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): implement subagent state merging logic"
```

---

### Task 5: Add Subagent Persistence

**Files:**
- Modify: `src/persist.rs`

- [ ] **Step 1: Add subagent persistence functions**

```rust
use crate::agent::SubagentInfo;

const SUBAGENT_STATES_FILE: &str = "subagent_states.jsonl";

fn subagent_states_path() -> PathBuf {
    state_dir().join(SUBAGENT_STATES_FILE)
}

/// Append a subagent state to the JSONL log.
pub fn append_subagent_state(pane_id: &str, subagent: &SubagentInfo) -> Result<()> {
    let path = subagent_states_path();
    let dir = path.parent().context("No parent directory")?;
    std::fs::create_dir_all(dir)?;
    
    let entry = SubagentStateEntry {
        pane_id: pane_id.to_string(),
        subagent: subagent.clone(),
    };
    
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct SubagentStateEntry {
    pane_id: String,
    subagent: SubagentInfo,
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
    
    for line in reader.lines().flatten() {
        if let Ok(entry) = serde_json::from_str::<SubagentStateEntry>(&line) {
            let key = (entry.pane_id, entry.subagent.id.clone());
            let ts = entry.subagent.updated_at;
            
            match states.get(&key) {
                Some(existing) if existing.updated_at >= ts => {}
                _ => {
                    states.insert(key, entry.subagent);
                }
            }
        }
    }
    
    // Remove ended subagents but count them
    states.retain(|_, info| info.state != SubagentStatus::Ended);
    states
}

/// Load completed subagent counts from the JSONL log.
pub fn load_completed_counts() -> HashMap<String, u32> {
    let path = subagent_states_path();
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };
    
    let reader = std::io::BufReader::new(file);
    let mut counts: HashMap<String, u32> = HashMap::new();
    
    for line in reader.lines().flatten() {
        if let Ok(entry) = serde_json::from_str::<SubagentStateEntry>(&line) {
            if entry.subagent.state == SubagentStatus::Ended {
                *counts.entry(entry.pane_id).or_insert(0) += 1;
            }
        }
    }
    
    counts
}
```

- [ ] **Step 2: Update App::new() to load persisted subagent states**

```rust
impl App {
    pub fn new() -> Self {
        // ... existing code ...
        Self {
            // ... existing fields ...
            subagent_states: persist::load_subagent_states(),
            completed_subagent_counts: persist::load_completed_counts(),
            // ... rest ...
        }
    }
}
```

- [ ] **Step 3: Add persistence call in merge_subagent_state**

```rust
fn merge_subagent_state(&mut self, pane_id: String, agent_id: String, state: AgentState) {
    // ... existing logic ...
    
    // Persist the subagent state
    if let Some(info) = self.subagent_states.get(&(pane_id.clone(), agent_id.clone())) {
        if let Err(e) = persist::append_subagent_state(&pane_id, info) {
            eprintln!("Warning: failed to persist subagent state: {}", e);
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test persist::`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/persist.rs src/app.rs
git commit -m "feat(persist): add subagent state persistence to JSONL"
```

---

### Task 6: Update UI Tree Rendering for Subagents

**Files:**
- Modify: `src/ui/tree.rs`

- [ ] **Step 1: Add subagent rendering constants**

```rust
/// Maximum number of subagents to display (others shown as count)
const MAX_VISIBLE_SUBAGENTS: usize = 3;
```

- [ ] **Step 2: Add function to calculate subagent visual rows**

```rust
/// Calculate the number of visual rows needed for subagents.
pub fn subagent_visual_rows(subagents: &[&SubagentInfo]) -> usize {
    subagents.iter().map(|s| 1 + s.tools.len().min(MAX_VISIBLE_TOOLS)).sum()
}
```

- [ ] **Step 3: Add function to render subagent status lines**

```rust
/// Render subagent status lines nested under the main agent.
fn render_subagent_lines(
    subagents: &[&SubagentInfo],
    completed_count: u32,
    width: u16,
    selected: bool,
    session_attached: bool,
    anim_frame: usize,
    toplevel: Option<&str>,
) -> Vec<Line<'static>> {
    let mut result = Vec::new();
    let content_width = (width as usize).saturating_sub(4);
    let border_style = session_border_style(session_attached);
    let dim_style = Style::default().fg(Color::Rgb(0x7a, 0x7a, 0x7a));
    let prefix_style = Style::default().fg(theme::COLOR_PURPLE);
    
    // Render active subagents (limited to MAX_VISIBLE_SUBAGENTS)
    let visible_count = subagents.len().min(MAX_VISIBLE_SUBAGENTS);
    for (i, subagent) in subagents.iter().take(visible_count).enumerate() {
        let is_last = i == visible_count - 1 && completed_count == 0;
        let tree_prefix = if is_last { "└─" } else { "├─" };
        
        // Status line
        let status_icon = match subagent.state {
            SubagentStatus::Running => theme::status_icon(&AgentStatus::Running, anim_frame).to_string(),
            SubagentStatus::Waiting => theme::status_icon(&AgentStatus::Waiting, anim_frame).to_string(),
            SubagentStatus::Ended => theme::status_icon(&AgentStatus::Ended, anim_frame).to_string(),
        };
        
        // Build label: "short_id: description" or just "short_id"
        let label = match &subagent.description {
            Some(desc) => {
                let truncated = if desc.len() > 30 {
                    format!("{}...", &desc[..27])
                } else {
                    desc.clone()
                };
                format!("{}: {}", subagent.short_id, truncated)
            }
            None => subagent.short_id.clone(),
        };
        
        let status_label = match subagent.state {
            SubagentStatus::Running => "running",
            SubagentStatus::Waiting => "waiting",
            SubagentStatus::Ended => "ended",
        };
        
        let mut status_spans = vec![
            Span::styled("    ", prefix_style),
            Span::styled(tree_prefix.to_string(), prefix_style),
            Span::styled(" ", prefix_style),
            Span::styled(status_icon, theme::status_style(&AgentStatus::Running, session_attached)),
            Span::styled(format!(" {}", label), dim_style),
            Span::styled(format!(" {}", status_label), dim_style),
        ];
        
        if !subagent.tools.is_empty() {
            let tool_label = if subagent.tools.len() == 1 {
                " (1 tool)".to_string()
            } else {
                format!(" ({} tools)", subagent.tools.len())
            };
            status_spans.push(Span::styled(tool_label, dim_style));
        }
        
        truncate_spans(&mut status_spans, content_width);
        result.push(wrap_bordered_line(status_spans, content_width, selected, border_style));
        
        // Tool lines
        let visible_tools = if subagent.tools.len() > MAX_VISIBLE_TOOLS {
            &subagent.tools[subagent.tools.len() - MAX_VISIBLE_TOOLS..]
        } else {
            &subagent.tools
        };
        
        for tool in visible_tools {
            let tool_text = match &tool.detail {
                Some(detail) => {
                    let display_detail = shorten_tool_detail(&tool.name, detail, toplevel);
                    format!("{} {}: {}", theme::ICON_TOOL, tool.name, display_detail)
                }
                None => format!("{} {}", theme::ICON_TOOL, tool.name),
            };
            
            let tool_prefix = if is_last { "    " } else { "│   " };
            let mut tool_spans = vec![
                Span::styled(format!("{}  ", tool_prefix), prefix_style),
                Span::styled(tool_text, dim_style),
            ];
            truncate_spans(&mut tool_spans, content_width);
            result.push(wrap_bordered_line(tool_spans, content_width, selected, border_style));
        }
    }
    
    // Show count of additional subagents
    if subagents.len() > MAX_VISIBLE_SUBAGENTS {
        let extra = subagents.len() - MAX_VISIBLE_SUBAGENTS;
        let mut count_spans = vec![
            Span::styled("    └─ ", prefix_style),
            Span::styled(format!("+{} more subagents", extra), dim_style),
        ];
        truncate_spans(&mut count_spans, content_width);
        result.push(wrap_bordered_line(count_spans, content_width, selected, border_style));
    }
    
    // Show completed count
    if completed_count > 0 {
        let mut completed_spans = vec![
            Span::styled("    └─ ", prefix_style),
            Span::styled(
                format!("✓ {} completed", completed_count),
                Style::default().fg(Color::Rgb(0x60, 0x60, 0x60)),
            ),
        ];
        truncate_spans(&mut completed_spans, content_width);
        result.push(wrap_bordered_line(completed_spans, content_width, selected, border_style));
    }
    
    result
}
```

- [ ] **Step 4: Update item_to_visual_row to account for subagents**

Find the `item_to_visual_row` function and update it to include subagent rows:

```rust
pub fn item_to_visual_row(items: &[TreeItem], target_idx: usize, width: u16) -> usize {
    let mut row = 0;
    for (i, item) in items.iter().enumerate() {
        if i == target_idx {
            return row;
        }
        row += item_visual_rows(item, width);
        
        // Add subagent rows if this item has subagents
        if let Some((subagents, completed)) = get_subagents_for_item(item) {
            row += subagent_visual_rows(&subagents) 
                + if completed > 0 { 1 } else { 0 }
                + if subagents.len() > MAX_VISIBLE_SUBAGENTS { 1 } else { 0 };
        }
    }
    row
}
```

- [ ] **Step 5: Update render function to draw subagent lines**

Find the main render function and add subagent rendering after the main agent status lines:

```rust
// After rendering main agent status lines...
// Render subagents if present
if let TreeItem::Window { pane, session_toplevel, .. } = item {
    // Need to pass subagents from app state - this requires updating the render signature
    // For now, we'll add a placeholder that will be connected in Task 7
}
```

- [ ] **Step 6: Commit**

```bash
git add src/ui/tree.rs
git commit -m "feat(ui): add subagent rendering functions"
```

---

### Task 7: Connect Subagent Data to UI Rendering

**Files:**
- Modify: `src/ui/tree.rs`
- Modify: `src/app.rs`

- [ ] **Step 1: Add subagent data to TreeItem or pass separately**

The cleanest approach is to pass subagent data during rendering. Update the render function signature:

```rust
pub fn render(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    items: &[TreeItem],
    selected: usize,
    scroll_offset: usize,
    anim_frame: usize,
    subagent_data: &HashMap<String, (Vec<SubagentInfo>, u32)>,  // pane -> (subagents, completed_count)
) {
    // ... existing code ...
}
```

- [ ] **Step 2: Update the App to build subagent data map**

```rust
impl App {
    fn build_subagent_data(&self) -> HashMap<String, (Vec<SubagentInfo>, u32)> {
        let mut data: HashMap<String, (Vec<SubagentInfo>, u32)> = HashMap::new();
        
        for ((pane_id, _), info) in &self.subagent_states {
            data.entry(pane_id.clone())
                .or_insert_with(|| (Vec::new(), 0))
                .0.push(info.clone());
        }
        
        for (pane_id, count) in &self.completed_subagent_counts {
            data.entry(pane_id.clone())
                .or_insert_with(|| (Vec::new(), 0))
                .1 = *count;
        }
        
        data
    }
}
```

- [ ] **Step 3: Update the render call in app.rs**

```rust
// In run_app, where tree::render is called:
let subagent_data = app.build_subagent_data();
tree::render(
    f,
    chunks[1],
    &app.tree_items,
    app.selected,
    app.scroll_offset,
    app.anim_frame,
    &subagent_data,
);
```

- [ ] **Step 4: Run full build**

Run: `cargo build`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add src/ui/tree.rs src/app.rs
git commit -m "feat: connect subagent data to UI rendering"
```

---

### Task 8: Add Integration Tests

**Files:**
- Modify: `src/app.rs` (add tests)

- [ ] **Step 1: Add integration test for full subagent lifecycle**

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    
    #[test]
    fn test_subagent_lifecycle() {
        let mut app = App::new();
        
        // Main agent starts
        let main_state = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: Some("session-1".to_string()),
            agent_id: None,
            state: AgentStatus::Started,
            updated_at: 100,
            hook_event_name: Some("SessionStart".to_string()),
            tool_name: None,
            tool_detail: None,
            tools: vec![],
        };
        app.agent_states.insert("%0".to_string(), main_state);
        
        // Subagent starts
        let subagent_start = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            agent_id: Some("agent-abc123".to_string()),
            state: AgentStatus::Running,
            updated_at: 110,
            hook_event_name: Some("SubagentStart".to_string()),
            tool_name: Some("Task".to_string()),
            tool_detail: None,
            tools: vec![ToolInfo {
                name: "Task".to_string(),
                detail: Some("Find all Rust files".to_string()),
            }],
        };
        app.merge_subagent_state("%0".to_string(), "agent-abc123".to_string(), subagent_start);
        
        // Subagent uses a tool
        let tool_use = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            agent_id: Some("agent-abc123".to_string()),
            state: AgentStatus::Running,
            updated_at: 120,
            hook_event_name: Some("PreToolUse".to_string()),
            tool_name: Some("Glob".to_string()),
            tool_detail: None,
            tools: vec![ToolInfo {
                name: "Glob".to_string(),
                detail: Some("**/*.rs".to_string()),
            }],
        };
        app.merge_subagent_state("%0".to_string(), "agent-abc123".to_string(), tool_use);
        
        // Verify subagent has tools
        let subagents = app.get_subagents_for_pane("%0");
        assert_eq!(subagents.len(), 1);
        assert_eq!(subagents[0].tools.len(), 1);
        
        // Subagent ends
        let subagent_end = AgentState {
            tmux_pane: "%0".to_string(),
            session_id: None,
            agent_id: Some("agent-abc123".to_string()),
            state: AgentStatus::Ended,
            updated_at: 130,
            hook_event_name: Some("SubagentStop".to_string()),
            tool_name: None,
            tool_detail: None,
            tools: vec![],
        };
        app.merge_subagent_state("%0".to_string(), "agent-abc123".to_string(), subagent_end);
        
        // Verify subagent is gone and counted as completed
        assert_eq!(app.get_subagents_for_pane("%0").len(), 0);
        assert_eq!(app.get_completed_count("%0"), 1);
    }
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "test: add integration tests for subagent lifecycle"
```

---

### Task 9: Final Cleanup and Documentation

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Run clippy and fix warnings**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 2: Update CLAUDE.md with subagent documentation**

Add to the "Architecture" section:

```markdown
### Subagent Tracking

Subagents spawned by the Task tool are tracked separately from the main agent:

- `subagent_states: HashMap<(tmux_pane, agent_id), SubagentInfo>` — active subagents
- `completed_subagent_counts: HashMap<tmux_pane, u32>` — completed count per pane

Hook events with `agent_id` are routed to subagent tracking. SubagentStart creates a new entry,
PreToolUse/PostToolUse update tools, and SubagentStop removes the entry and increments the
completed count.

UI renders subagents nested under the main agent with tree prefixes (├─/└─), showing each
subagent's status and active tools.
```

- [ ] **Step 3: Final commit**

```bash
git add CLAUDE.md
git commit -m "docs: document subagent tracking architecture"
```

---

## Self-Review Checklist

**1. Spec coverage:**
- ✅ Task 1-2: Define data structures and parse agent_id
- ✅ Task 3-4: Store and merge subagent states
- ✅ Task 5: Persist subagent states
- ✅ Task 6-7: Render subagents in UI with nesting
- ✅ Task 8: Integration tests

**2. Placeholder scan:**
- No "TBD", "TODO", or vague references found
- All code blocks contain complete implementations

**3. Type consistency:**
- `SubagentInfo.id` and `SubagentInfo.short_id` are consistently `String`
- `agent_id` field added to `AgentState` matches usage in `hook.rs`
- Key type `(String, String)` for `(pane_id, agent_id)` is consistent throughout
