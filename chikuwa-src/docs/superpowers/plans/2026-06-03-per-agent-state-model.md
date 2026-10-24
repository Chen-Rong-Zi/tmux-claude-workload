# Per-Agent State Model Refactor

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the lossy `AgentState` with per-agent state types that preserve each agent's full data model, unified via an `AgentData` enum and a `AgentView` trait for UI rendering.

**Architecture:** Each agent (Claude, OpenCode) gets its own complete state struct with all fields it actually has. These are wrapped in `AgentData` enum. UI reads state through `AgentView` trait which extracts common rendering info. Tool matching uses `ToolKey` enum (Claude uses `tool_use_id`, OpenCode uses `name+detail`). The `App` merge logic becomes per-variant, eliminating the fragile name+detail matching.

**Tech Stack:** Rust, serde, ratatui

---

## File Structure

| File | Responsibility |
|---|---|
| `src/agent/state.rs` | Core types: `AgentData` enum, `AgentState`, `AgentView` trait, `ToolKey`, `ActiveTool`, per-agent structs |
| `src/agent/claude.rs` (new) | `ClaudeState`, `ClaudeActiveTool`, merge logic |
| `src/agent/opencode_state.rs` (new) | `OpenCodeState`, `OpenCodeActiveTool`, merge logic |
| `src/agent/parser.rs` | Updated parsers that produce per-agent state types |
| `src/agent/mod.rs` | Updated exports |
| `src/app.rs` | Updated `AgentStateUpdate` handler — dispatches to per-agent merge |
| `src/hook.rs` | Updated to use new parser output types |
| `src/opencode.rs` | Updated to use new parser output types |
| `src/ipc.rs` | No change (serializes `AgentState` which contains `AgentData`) |
| `src/persist.rs` | Updated `AgentState` construction in tests |
| `src/ui/tree.rs` | Uses `AgentView` trait instead of direct field access |
| `src/ui/office.rs` | Uses `AgentView` trait instead of direct field access |
| `plugins/chikuwa.ts` | Updated to send `AgentData::OpenCode` tagged JSON |

---

### Task 1: Define new state types in `agent/state.rs`

**Files:**
- Modify: `src/agent/state.rs`

Replace the current `AgentState` and `ToolInfo` with per-agent state types.

- [ ] **Step 1: Add new type definitions before the existing `AgentState`**

Add these types at the top of `state.rs` (after imports):

```rust
use serde::{Deserialize, Serialize};

/// Unique identifier for an in-flight tool call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolKey {
    /// Claude Code: exact match via tool_use_id
    Claude { tool_use_id: String },
    /// OpenCode: no unique ID, approximate match via name+detail
    OpenCode { name: String, detail: Option<String> },
}

/// A tool call that is currently in progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTool {
    pub key: ToolKey,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
}

/// Which agent produced this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    Claude,
    OpenCode,
}

/// Read-only view of agent state for UI rendering.
pub trait AgentView {
    fn status(&self) -> AgentStatus;
    fn source(&self) -> AgentSource;
    fn session_id(&self) -> Option<&str>;
    fn agent_id(&self) -> Option<&str>;
    fn event_label(&self) -> &str;
    fn event_emoji(&self) -> Option<&str>;
    fn active_tools(&self) -> &[ActiveTool];
    fn current_tool_name(&self) -> Option<&str>;
    fn current_tool_detail(&self) -> Option<&str>;
    fn failure_detail(&self) -> Option<&str>;
}
```

- [ ] **Step 2: Update `AgentState` to wrap `AgentData`**

Replace the current `AgentState` struct with:

```rust
/// Per-agent state data, tagged by source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentData {
    Claude(ClaudeState),
    OpenCode(OpenCodeState),
}

/// Top-level agent state tracked by the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub tmux_pane: String,
    pub updated_at: u64,
    pub data: AgentData,
}

impl AgentState {
    pub fn new(tmux_pane: String, data: AgentData) -> Self {
        Self {
            tmux_pane,
            updated_at: now(),
            data,
        }
    }

    pub fn status(&self) -> AgentStatus {
        match &self.data {
            AgentData::Claude(c) => c.status,
            AgentData::OpenCode(o) => o.status,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.session_id.as_deref(),
            AgentData::OpenCode(o) => o.session_id.as_deref(),
        }
    }

    pub fn agent_id(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.agent_id.as_deref(),
            AgentData::OpenCode(_) => None,
        }
    }

    pub fn source(&self) -> AgentSource {
        match &self.data {
            AgentData::Claude(_) => AgentSource::Claude,
            AgentData::OpenCode(_) => AgentSource::OpenCode,
        }
    }
}

impl AgentView for AgentState {
    fn status(&self) -> AgentStatus { self.status() }
    fn source(&self) -> AgentSource { self.source() }
    fn session_id(&self) -> Option<&str> { self.session_id() }
    fn agent_id(&self) -> Option<&str> { self.agent_id() }
    fn event_label(&self) -> &str {
        match &self.data {
            AgentData::Claude(c) => c.hook_event_name.as_deref().unwrap_or("Agent"),
            AgentData::OpenCode(o) => o.event_type.as_deref().unwrap_or("Agent"),
        }
    }
    fn event_emoji(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.event_emoji.as_deref(),
            AgentData::OpenCode(o) => o.event_emoji.as_deref(),
        }
    }
    fn active_tools(&self) -> &[ActiveTool] {
        match &self.data {
            AgentData::Claude(c) => &c.active_tools,
            AgentData::OpenCode(o) => &o.active_tools,
        }
    }
    fn current_tool_name(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.tool_name.as_deref(),
            AgentData::OpenCode(o) => o.tool_name.as_deref(),
        }
    }
    fn current_tool_detail(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.tool_detail.as_deref(),
            AgentData::OpenCode(o) => o.tool_detail.as_deref(),
        }
    }
    fn failure_detail(&self) -> Option<&str> {
        match &self.data {
            AgentData::Claude(c) => c.failure_detail.as_deref(),
            AgentData::OpenCode(o) => None,
        }
    }
}
```

- [ ] **Step 3: Remove old `ToolInfo` struct**

Delete the `ToolInfo` struct and its `#[derive]` attributes. The `ActiveTool` replaces it.

- [ ] **Step 4: Run `cargo check` to see all compile errors**

Run: `cargo check 2>&1 | head -50`
Expected: Many errors from code referencing old `ToolInfo` and `AgentState` fields. This is expected — we'll fix them in subsequent tasks.

- [ ] **Step 5: Commit (broken state is OK, this is a refactoring branch)**

```bash
git add src/agent/state.rs
git commit -m "refactor: define per-agent state types (AgentData, ToolKey, ActiveTool)"
```

---

### Task 2: Create `agent/claude.rs` with `ClaudeState` and merge logic

**Files:**
- Create: `src/agent/claude.rs`

- [ ] **Step 1: Create `src/agent/claude.rs`**

```rust
use serde::{Deserialize, Serialize};

use super::state::{ActiveTool, AgentStatus, ToolKey};

/// Full state from Claude Code hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Failure message from PostToolUseFailure
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
}

impl ClaudeState {
    /// Merge an incoming event into existing state, returning the new state.
    pub fn merge(incoming: ClaudeState, existing: &ClaudeState) -> ClaudeState {
        let event = incoming.hook_event_name.as_str();
        let is_silent = event == "PostToolUse";

        // Preserve session_id if incoming is None
        let session_id = incoming
            .session_id
            .or_else(|| existing.session_id.clone());

        // Merge active tools
        let active_tools = if incoming.status == AgentStatus::Running {
            match event {
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
                            // Match by ToolKey (exact match first)
                            let pos = tools
                                .iter()
                                .position(|t| t.key == removing.key)
                                .or_else(|| {
                                    // Fallback: match by name only
                                    tools.iter().position(|t| t.name == removing.name)
                                });
                            if let Some(pos) = pos {
                                tools.remove(pos);
                            }
                        }
                    }
                    tools
                }
                _ => existing.active_tools.clone(),
            }
        } else {
            Vec::new()
        };

        let mut merged = incoming;
        merged.session_id = session_id;
        merged.active_tools = active_tools;

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
```

- [ ] **Step 2: Commit**

```bash
git add src/agent/claude.rs
git commit -m "feat: add ClaudeState with merge logic"
```

---

### Task 3: Create `agent/opencode_state.rs` with `OpenCodeState` and merge logic

**Files:**
- Create: `src/agent/opencode_state.rs`

- [ ] **Step 1: Create `src/agent/opencode_state.rs`**

```rust
use serde::{Deserialize, Serialize};

use super::state::{ActiveTool, AgentStatus, ToolKey};

/// Full state from OpenCode hooks/plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub status: AgentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_tools: Vec<ActiveTool>,
    #[serde(default)]
    pub is_busy: bool,
}

impl OpenCodeState {
    /// Merge an incoming event into existing state, returning the new state.
    pub fn merge(incoming: OpenCodeState, existing: &OpenCodeState) -> OpenCodeState {
        let event = incoming.event_type.as_deref().unwrap_or("");

        // Preserve session_id if incoming is None
        let session_id = incoming
            .session_id
            .or_else(|| existing.session_id.clone());

        // Merge active tools
        let active_tools = match event {
            "tool.execute" | "tool.running" => {
                let mut tools = existing.active_tools.clone();
                for tool in &incoming.active_tools {
                    if !tools.iter().any(|t| t.key == tool.key) {
                        tools.push(tool.clone());
                    }
                }
                tools
            }
            "tool.completed" | "tool.error" => {
                let mut tools = existing.active_tools.clone();
                if let Some(removing) = incoming.active_tools.first() {
                    let pos = tools.iter().position(|t| t.key == removing.key);
                    if let Some(pos) = pos {
                        tools.remove(pos);
                    }
                }
                tools
            }
            "session.idle" | "session.deleted" => Vec::new(),
            _ => existing.active_tools.clone(),
        };

        let mut merged = incoming;
        merged.session_id = session_id;
        merged.active_tools = active_tools;
        merged
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/agent/opencode_state.rs
git commit -m "feat: add OpenCodeState with merge logic"
```

---

### Task 4: Update `agent/mod.rs` exports

**Files:**
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Add new module declarations and exports**

```rust
pub mod claude;
pub mod opencode_state;
pub mod parser;
pub mod state;
pub mod subagent;

pub use claude::ClaudeState;
pub use opencode_state::OpenCodeState;
pub use parser::{ClaudeHookParser, HookParser, OpenCodeHookParser};
pub use state::{ActiveTool, AgentData, AgentSource, AgentState, AgentView, ToolKey};
pub use subagent::{SubagentInfo, SubagentStatus};
```

- [ ] **Step 2: Commit**

```bash
git add src/agent/mod.rs
git commit -m "refactor: update agent module exports"
```

---

### Task 5: Update `agent/parser.rs` — Claude parser produces `ClaudeState`

**Files:**
- Modify: `src/agent/parser.rs`

The parser now produces `AgentData::Claude(ClaudeState)` and `AgentData::OpenCode(OpenCodeState)` wrapped in `AgentState`, instead of setting flat fields on a generic `AgentState`.

- [ ] **Step 1: Update imports and `ClaudeHookInput`**

Replace the imports at the top of parser.rs:

```rust
use anyhow::{Context, Result};
use serde::Deserialize;

use super::claude::ClaudeState;
use super::opencode_state::OpenCodeState;
use super::state::{ActiveTool, AgentData, AgentState, AgentStatus, ToolKey};
```

Update `ClaudeHookInput` to include `tool_use_id`:

```rust
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
```

- [ ] **Step 2: Update `ClaudeHookParser::parse` to produce `AgentData::Claude`**

Replace the body of `ClaudeHookParser::parse` (after `state.tool_detail = ...`) with:

```rust
        // Determine display mode based on event type
        let display = match event_name.as_str() {
            "PostToolUse" => DisplayMode::Silent,
            "PostToolUseFailure" => DisplayMode::Show,
            _ => DisplayMode::Show,
        };

        // Extract tool_detail using the existing extract_tool_detail function
        let tool_detail = extract_event_detail(&event_name, &input).or_else(|| {
            input.tool_name.as_ref().and_then(|name| {
                input.tool_input.as_ref().and_then(|inp| extract_tool_detail(name, inp))
            })
        });

        // Build active_tools for this event
        let active_tools = if let Some(ref name) = input.tool_name {
            let tool_use_id = input.tool_use_id.clone().unwrap_or_else(|| {
                // Fallback: generate a synthetic key from name + input hash
                format!("{}:{:x}", name, input.tool_input.as_ref().map(|v| v.to_string().len()).unwrap_or(0))
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
            session_id: input.session_id.clone(),
            agent_id: input.agent_id.clone(),
            status: mapping.status,
            hook_event_name: event_name.clone(),
            event_emoji: mapping.emoji.to_string(),
            tool_name: input.tool_name.clone(),
            tool_detail,
            active_tools,
            failure_detail,
        };

        let agent_state = AgentState::new(pane_id, AgentData::Claude(claude_state));

        Ok(ParseResult { state: agent_state, display })
```

- [ ] **Step 3: Update `OpenCodeHookParser::parse` to produce `AgentData::OpenCode`**

Replace the body of `OpenCodeHookParser::parse` with:

```rust
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
            is_busy: status == AgentStatus::Running,
        };

        Ok(ParseResult {
            state: AgentState::new(pane_id, AgentData::OpenCode(opencode_state)),
            display: DisplayMode::Show,
        })
```

- [ ] **Step 4: Update test imports and assertions**

Update all parser test code to use `AgentData::Claude` / `AgentData::OpenCode` variants. For example:

```rust
fn test_claude_hook_input_deserialize() {
    let json = r#"{"hook_event_name":"SessionStart","session_id":"abc123"}"#;
    let parser = ClaudeHookParser;
    let result = parser.parse("%0".to_string(), json).unwrap();
    assert!(result.display == DisplayMode::Show);
    let data = match &result.state.data {
        AgentData::Claude(c) => c,
        _ => panic!("expected Claude data"),
    };
    assert_eq!(data.status, AgentStatus::Started);
    assert_eq!(data.event_emoji, "🚀");
    assert_eq!(data.session_id.as_deref(), Some("abc123"));
}
```

Apply similar pattern to all parser tests. Each test extracts the inner `ClaudeState` or `OpenCodeState` and asserts on its fields.

- [ ] **Step 5: Run tests**

Run: `cargo test agent::parser 2>&1`
Expected: Some tests may fail due to incomplete migration — fix remaining field access patterns.

- [ ] **Step 6: Commit**

```bash
git add src/agent/parser.rs
git commit -m "refactor: parsers produce per-agent state types"
```

---

### Task 6: Update `app.rs` — per-agent merge dispatch

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Update `AgentStateUpdate` handler**

Replace the main agent merge logic (the `else` branch after subagent check) with per-variant dispatch:

```rust
AppEvent::AgentStateUpdate(state) => {
    if let Some(ref agent_id) = state.agent_id() {
        // Subagent event
        let pane_id = state.tmux_pane.clone();
        let agent_id = agent_id.to_string();
        app.merge_subagent_state(pane_id, agent_id, state);
    } else {
        // Main agent event — dispatch by agent source
        if state.status() == AgentStatus::Ended {
            app.agent_states.remove(&state.tmux_pane);
        } else if let Some(existing) = app.agent_states.get(&state.tmux_pane) {
            // Only merge same-source states
            let merged = if state.source() == existing.source() {
                match (&state.data, &existing.data) {
                    (AgentData::Claude(incoming), AgentData::Claude(existing)) => {
                        let merged_claude = ClaudeState::merge(
                            incoming.clone(),
                            existing,
                        );
                        let mut merged = state;
                        merged.data = AgentData::Claude(merged_claude);
                        merged
                    }
                    (AgentData::OpenCode(incoming), AgentData::OpenCode(existing)) => {
                        let merged_opencode = OpenCodeState::merge(
                            incoming.clone(),
                            existing,
                        );
                        let mut merged = state;
                        merged.data = AgentData::OpenCode(merged_opencode);
                        merged
                    }
                    _ => state, // Different source: replace
                }
            } else {
                state // Source changed: replace entirely
            };
            app.agent_states.insert(merged.tmux_pane.clone(), merged);
        } else {
            app.agent_states.insert(state.tmux_pane.clone(), state);
        }
    }
    app.merge_agent_states();
}
```

- [ ] **Step 2: Update subagent merge to use `AgentView`**

The `merge_subagent_state` method currently reads `state.agent_id`, `state.tools`, etc. Update to use `AgentView` trait methods and `active_tools()` instead of `tools`.

- [ ] **Step 3: Run `cargo check`**

Run: `cargo check 2>&1 | head -80`
Expected: Errors in UI files (tree.rs, office.rs) referencing old fields. This is expected.

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "refactor: per-agent merge dispatch in app"
```

---

### Task 7: Update UI — use `AgentView` trait

**Files:**
- Modify: `src/ui/tree.rs`
- Modify: `src/ui/office.rs`

- [ ] **Step 1: Update `tree.rs` to use `AgentView`**

Replace all direct field access on `AgentState` with `AgentView` trait method calls:

- `agent.state` → `agent.status()`
- `agent.tools` → `agent.active_tools()`
- `agent.tool_name` → `agent.current_tool_name()`
- `agent.tool_detail` → `agent.current_tool_detail()`
- `agent.failure_detail` → `agent.failure_detail()`
- `agent.event_emoji` → `agent.event_emoji()`
- `agent.hook_event_name` → `agent.event_label()`

The `agent_status_visual_rows` function:
```rust
fn agent_status_visual_rows(item: &TreeItem) -> usize {
    let agent: &dyn AgentView = match item {
        TreeItem::Window { window, .. } => match window.panes.first().and_then(|p| p.agent_state.as_ref()) {
            Some(a) => a,
            None => return 0,
        },
        TreeItem::Pane { pane, .. } => match pane.agent_state.as_ref() {
            Some(a) => a,
            None => return 0,
        },
        _ => return 0,
    };
    1 + agent.active_tools().len().min(MAX_VISIBLE_TOOLS)
        + if agent.failure_detail().is_some() { 1 } else { 0 }
}
```

Similarly update `render_bordered_agent_status_sub_lines` to take `&dyn AgentView` and use trait methods.

- [ ] **Step 2: Update `office.rs` to use `AgentView`**

Same pattern — replace direct field access with trait method calls. The `agent` parameter in `render_agent_room` and related functions should use `&dyn AgentView`.

- [ ] **Step 3: Run `cargo check`**

Run: `cargo check 2>&1`
Expected: Errors in persist.rs and test files. Fix those.

- [ ] **Step 4: Commit**

```bash
git add src/ui/tree.rs src/ui/office.rs
git commit -m "refactor: UI uses AgentView trait for rendering"
```

---

### Task 8: Update `persist.rs` and all remaining compilation errors

**Files:**
- Modify: `src/persist.rs`
- Modify: `src/hook.rs`
- Modify: `src/opencode.rs`

- [ ] **Step 1: Update `persist.rs` test AgentState constructions**

Replace all `AgentState { tmux_pane, session_id, ... tools: Vec::new() }` with:

```rust
AgentState::new(
    "%0".to_string(),
    AgentData::Claude(ClaudeState {
        session_id: Some("s1".to_string()),
        agent_id: None,
        status: AgentStatus::Running,
        hook_event_name: "PreToolUse".to_string(),
        event_emoji: "🔧".to_string(),
        tool_name: None,
        tool_detail: None,
        active_tools: Vec::new(),
        failure_detail: None,
    }),
)
```

- [ ] **Step 2: Update `hook.rs` and `opencode.rs`**

These files use `DisplayMode` from parser — they should still work since `ParseResult` still has `display: DisplayMode`. Verify they compile.

- [ ] **Step 3: Run `cargo check`**

Run: `cargo check 2>&1`
Expected: Clean compilation (possibly with warnings).

- [ ] **Step 4: Commit**

```bash
git add src/persist.rs src/hook.rs src/opencode.rs
git commit -m "refactor: update persist, hook, opencode for new state types"
```

---

### Task 9: Update OpenCode plugin (`plugins/chikuwa.ts`)

**Files:**
- Modify: `plugins/chikuwa.ts`

- [ ] **Step 1: Update `AgentState` interface and `sendState` calls**

The plugin now needs to send `AgentData::OpenCode` tagged JSON. Update the `AgentState` interface:

```typescript
interface OpenCodeActiveTool {
    key: { type: "open_code"; name: string; detail?: string };
    name: string;
    detail?: string;
    failure_detail?: string;
}

interface OpenCodeState {
    session_id?: string;
    status: "started" | "running" | "waiting" | "permission" | "ended";
    event_type?: string;
    event_emoji?: string;
    tool_name?: string;
    tool_detail?: string;
    active_tools: OpenCodeActiveTool[];
    is_busy: boolean;
}

interface AgentStateMessage {
    tmux_pane: string;
    updated_at: number;
    data: {
        type: "open_code";
        ...OpenCodeState;
    };
}
```

Update all `sendState` calls to construct the new format with `ToolKey::OpenCode` keyed active tools.

- [ ] **Step 2: Commit**

```bash
git add plugins/chikuwa.ts
git commit -m "refactor: OpenCode plugin sends per-agent tagged state"
```

---

### Task 10: Format, lint, test, fix all remaining issues

**Files:**
- All modified files

- [ ] **Step 1: Run format**

Run: `cargo fmt`

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings 2>&1`
Fix any warnings.

- [ ] **Step 3: Run all tests**

Run: `cargo test 2>&1`
Fix any failing tests.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: format, lint, test fixes for per-agent state refactor"
```
