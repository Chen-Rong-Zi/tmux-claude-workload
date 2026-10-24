# Codex Hooks Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Codex CLI as a third agent source in chikuwa, with hook parser, state tracking, subagent support, and setup docs for all Codex hook events.

**Architecture:** Add Codex as a first-class `AgentSource`/`AgentData` variant while keeping source-specific parsing and merge logic isolated in `CodexHookParser` and `CodexState`. Reuse the existing `HookParser`, `AgentView`, IPC, persistence, and subagent rendering paths so the TUI continues to render a unified `AgentState` regardless of agent source.

**Tech Stack:** Rust, serde, serde_json, clap, Codex CLI hooks API

---

## Codex Hook Contract

Codex sends one JSON object to hook stdin. Common fields include `session_id`, `transcript_path`, `cwd`, `hook_event_name`, and `model`. Turn-scoped hooks also include `turn_id`; many events include `permission_mode`. Tool events include `tool_name`, `tool_use_id`, `tool_input`, and for `PostToolUse`, `tool_response`.

### Event Mapping

| Codex Event | AgentStatus | Emoji | Notes |
|---|---|---|---|
| `SessionStart` | `Started` | 🚀 | session begins/resumes/clears/compacts |
| `SubagentStart` | `Running` | 🤖 | creates/updates subagent state using `agent_id` |
| `PreToolUse` | `Running` | 🔧 | adds active tool by `tool_use_id` |
| `PermissionRequest` | `Permission` | 🔐 | approval needed |
| `PostToolUse` | `Running` | ✅ | silent visual update; removes active tool by `tool_use_id` |
| `PreCompact` | `Running` | 🗜️ | compaction starting |
| `PostCompact` | `Running` | 📦 | compaction finished |
| `UserPromptSubmit` | `Running` | 💭 | user prompt submitted |
| `SubagentStop` | `Ended` | 🏁 | removes subagent and increments completed count |
| `Stop` | `Waiting` | 💤 | main turn finished |

### File Inventory

| Action | File | Responsibility |
|---|---|---|
| Create | `src/agent/codex_state.rs` | Codex-specific state struct, merge logic, unit tests |
| Create | `src/codex_hook.rs` | `chikuwa codex-hook` handler: stdin JSON → IPC + persistence |
| Modify | `src/agent/mod.rs` | Export `codex_state` module and `CodexHookParser` |
| Modify | `src/agent/state.rs` | Add Codex variants and `AgentView` delegation |
| Modify | `src/agent/parser.rs` | Add Codex input schema, event mapping, parser, tool detail tests |
| Modify | `src/main.rs` | Add `codex-hook` CLI subcommand |
| Modify | `src/app.rs` | Route Codex subagent events through existing subagent tracker |
| Modify | `README.md` | Document Codex hook setup and event mapping |

---

### Task 1: Add `ToolKey::Codex`

**Files:**
- Modify: `src/agent/state.rs`

- [ ] **Step 1: Write failing ToolKey serialization test**

Add this test inside the existing `#[cfg(test)] mod tests` in `src/agent/state.rs`:

```rust
#[test]
fn test_codex_tool_key_serializes() {
    let key = ToolKey::Codex {
        tool_use_id: "call-123".to_string(),
    };

    let json = serde_json::to_string(&key).unwrap();

    assert_eq!(json, r#"{"type":"codex","tool_use_id":"call-123"}"#);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_codex_tool_key_serializes -v`

Expected: FAIL to compile with `no variant named Codex found for enum ToolKey`.

- [ ] **Step 3: Add the Codex variant**

In `src/agent/state.rs`, update `ToolKey`:

```rust
pub enum ToolKey {
    /// Claude Code: exact match via tool_use_id
    Claude { tool_use_id: String },
    /// Codex CLI: exact match via tool_use_id
    Codex { tool_use_id: String },
    /// OpenCode: no unique ID, approximate match via name+detail
    OpenCode {
        name: String,
        detail: Option<String>,
    },
}
```

- [ ] **Step 4: Run focused test**

Run: `cargo test test_codex_tool_key_serializes -v`

Expected: PASS.

- [ ] **Step 5: Run state tests**

Run: `cargo test agent::state -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/agent/state.rs
git commit -m "feat: add codex tool keys"
```

---

### Task 2: Create `CodexState` and merge behavior

**Files:**
- Create: `src/agent/codex_state.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Create failing tests and struct shell**

Create `src/agent/codex_state.rs` with the full test module and a minimal placeholder struct that will fail behavior assertions:

```rust
use serde::{Deserialize, Serialize};

use super::state::{ActiveTool, AgentStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub status: AgentStatus,
    pub hook_event_name: String,
    pub event_emoji: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_tools: Vec<ActiveTool>,
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
            failure_detail: None,
            turn_id: None,
            permission_mode: None,
            model: None,
            cwd: None,
            agent_type: None,
            transcript_path: None,
        }
    }

    pub fn merge(incoming: CodexState, _existing: &CodexState) -> CodexState {
        incoming
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::{ActiveTool, AgentStatus, ToolKey};

    fn make_state(event: &str, status: AgentStatus, tool_name: Option<&str>, tool_use_id: Option<&str>) -> CodexState {
        let mut state = CodexState::new(event, status, "🔧");
        state.tool_name = tool_name.map(str::to_string);
        if let (Some(name), Some(id)) = (tool_name, tool_use_id) {
            state.active_tools.push(ActiveTool {
                key: ToolKey::Codex { tool_use_id: id.to_string() },
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
        let incoming = make_state("PreToolUse", AgentStatus::Running, Some("Bash"), Some("call-1"));
        let existing = make_state("SessionStart", AgentStatus::Started, None, None);

        let merged = CodexState::merge(incoming, &existing);

        assert_eq!(merged.active_tools.len(), 1);
        assert_eq!(merged.active_tools[0].key, ToolKey::Codex { tool_use_id: "call-1".to_string() });
    }

    #[test]
    fn test_post_tool_use_removes_matching_tool_and_preserves_visual_state() {
        let mut existing = make_state("PreToolUse", AgentStatus::Running, Some("Bash"), Some("call-1"));
        existing.tool_detail = Some("ls".to_string());
        let incoming = make_state("PostToolUse", AgentStatus::Running, Some("Bash"), Some("call-1"));

        let merged = CodexState::merge(incoming, &existing);

        assert!(merged.active_tools.is_empty());
        assert_eq!(merged.hook_event_name, "PreToolUse");
        assert_eq!(merged.tool_detail.as_deref(), Some("ls"));
    }

    #[test]
    fn test_non_running_status_clears_active_tools() {
        let existing = make_state("PreToolUse", AgentStatus::Running, Some("Bash"), Some("call-1"));
        let incoming = make_state("Stop", AgentStatus::Waiting, None, None);

        let merged = CodexState::merge(incoming, &existing);

        assert!(merged.active_tools.is_empty());
    }

    #[test]
    fn test_serialization_roundtrip_preserves_codex_fields() {
        let mut state = make_state("PreToolUse", AgentStatus::Running, Some("Bash"), Some("call-1"));
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
```

- [ ] **Step 2: Export module**

In `src/agent/mod.rs`, add:

```rust
pub mod codex_state;
```

- [ ] **Step 3: Run tests to verify RED**

Run: `cargo test codex_state -v`

Expected: FAIL; specifically preservation/removal tests fail because `merge()` returns `incoming` unchanged.

- [ ] **Step 4: Implement merge logic**

Replace `CodexState::merge` with:

```rust
pub fn merge(incoming: CodexState, existing: &CodexState) -> CodexState {
    let event = incoming.hook_event_name.clone();
    let is_silent = event == "PostToolUse";

    let session_id = incoming
        .session_id
        .clone()
        .or_else(|| existing.session_id.clone());
    let agent_id = incoming.agent_id.clone().or_else(|| existing.agent_id.clone());

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
                        tools.remove(pos);
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
    merged.agent_id = agent_id;
    merged.active_tools = active_tools;

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
```

- [ ] **Step 5: Run Codex state tests to verify GREEN**

Run: `cargo test codex_state -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/agent/codex_state.rs src/agent/mod.rs
git commit -m "feat: add codex agent state"
```

---

### Task 3: Wire Codex into `AgentData` and `AgentView`

**Files:**
- Modify: `src/agent/state.rs`

- [ ] **Step 1: Write failing AgentData roundtrip test**

Add to `src/agent/state.rs` tests:

```rust
#[test]
fn test_codex_agent_state_roundtrip_json() {
    let state = AgentState::new(
        "%1".to_string(),
        AgentData::Codex(super::codex_state::CodexState::new(
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
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test test_codex_agent_state_roundtrip_json -v`

Expected: FAIL to compile because `AgentData::Codex` and `AgentSource::Codex` do not exist.

- [ ] **Step 3: Add Codex variants**

Update enums in `src/agent/state.rs`:

```rust
pub enum AgentSource {
    Claude,
    OpenCode,
    Codex,
}

pub enum AgentData {
    Claude(super::claude::ClaudeState),
    OpenCode(super::opencode_state::OpenCodeState),
    Codex(super::codex_state::CodexState),
}
```

- [ ] **Step 4: Update `AgentData::merge`**

Add the Codex arm:

```rust
(AgentData::Codex(in_c), AgentData::Codex(ex_c)) => {
    AgentData::Codex(super::codex_state::CodexState::merge(in_c.clone(), ex_c))
}
```

- [ ] **Step 5: Update `AgentState` methods and `AgentView` delegation**

Update every `match &self.data` in `AgentState`/`AgentView` with Codex arms:

```rust
AgentData::Codex(c) => c.status,
AgentData::Codex(c) => c.session_id.as_deref(),
AgentData::Codex(c) => c.agent_id.as_deref(),
AgentData::Codex(_) => AgentSource::Codex,
AgentData::Codex(c) => &c.hook_event_name,
AgentData::Codex(c) => Some(&c.event_emoji),
AgentData::Codex(c) => &c.active_tools,
AgentData::Codex(c) => c.tool_name.as_deref(),
AgentData::Codex(c) => c.tool_detail.as_deref(),
AgentData::Codex(c) => c.failure_detail.as_deref(),
```

- [ ] **Step 6: Run focused test**

Run: `cargo test test_codex_agent_state_roundtrip_json -v`

Expected: PASS.

- [ ] **Step 7: Run all agent tests**

Run: `cargo test agent:: -v`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/agent/state.rs
git commit -m "feat: wire codex into agent state"
```

---

### Task 4: Add `CodexHookParser`

**Files:**
- Modify: `src/agent/parser.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Add failing parser tests**

Add these tests inside `src/agent/parser.rs` test module:

```rust
fn codex_input(event: &str) -> String {
    format!(
        r#"{{"hook_event_name":"{}","session_id":"sess-1","cwd":"/repo","turn_id":"turn-1","model":"o3","transcript_path":"/tmp/transcript.jsonl"}}"#,
        event
    )
}

#[test]
fn test_codex_hook_session_start() {
    let parser = CodexHookParser;
    let result = parser.parse("%0".to_string(), &codex_input("SessionStart")).unwrap();

    assert_eq!(result.display, DisplayMode::Show);
    assert_eq!(result.state.status(), AgentStatus::Started);
    assert_eq!(result.state.source(), AgentSource::Codex);
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
    assert_eq!(result.state.active_tools()[0].key, ToolKey::Codex { tool_use_id: "call-1".to_string() });
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
    let result = parser.parse("%0".to_string(), &codex_input("UnknownEvent")).unwrap();

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
        assert_eq!(result.display, DisplayMode::Show, "{event} should be displayed");
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
```

- [ ] **Step 2: Run parser test to verify RED**

Run: `cargo test test_codex_hook_session_start -v`

Expected: FAIL to compile because `CodexHookParser` does not exist.

- [ ] **Step 3: Add imports**

At the top of `src/agent/parser.rs`, add:

```rust
use super::codex_state::CodexState;
```

In test module imports, include:

```rust
use crate::agent::state::{AgentSource, ToolKey};
```

- [ ] **Step 4: Add Codex input type and event mapping**

Add near the parser structs in `src/agent/parser.rs`:

```rust
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
    trigger: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

fn codex_event_mapping(event: &str) -> Option<EventMapping> {
    match event {
        "SessionStart" => Some(EventMapping { status: AgentStatus::Started, emoji: "🚀" }),
        "SubagentStart" => Some(EventMapping { status: AgentStatus::Running, emoji: "🤖" }),
        "PreToolUse" => Some(EventMapping { status: AgentStatus::Running, emoji: "🔧" }),
        "PermissionRequest" => Some(EventMapping { status: AgentStatus::Permission, emoji: "🔐" }),
        "PostToolUse" => Some(EventMapping { status: AgentStatus::Running, emoji: "✅" }),
        "PreCompact" => Some(EventMapping { status: AgentStatus::Running, emoji: "🗜️" }),
        "PostCompact" => Some(EventMapping { status: AgentStatus::Running, emoji: "📦" }),
        "UserPromptSubmit" => Some(EventMapping { status: AgentStatus::Running, emoji: "💭" }),
        "SubagentStop" => Some(EventMapping { status: AgentStatus::Ended, emoji: "🏁" }),
        "Stop" => Some(EventMapping { status: AgentStatus::Waiting, emoji: "💤" }),
        _ => None,
    }
}
```

- [ ] **Step 5: Add Codex tool detail extraction**

```rust
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
```

- [ ] **Step 6: Implement `CodexHookParser`**

```rust
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

        let active_tools = match (event_name.as_str(), tool_name.as_ref(), input.tool_use_id.as_ref()) {
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
            failure_detail: None,
            turn_id: input.turn_id,
            permission_mode: input.permission_mode,
            model: input.model,
            cwd: input.cwd,
            agent_type: input.agent_type,
            transcript_path: input
                .agent_transcript_path
                .or(input.transcript_path)
                .or(input.source),
        };

        Ok(ParseResult {
            state: AgentState::new(pane_id, AgentData::Codex(state)),
            display: DisplayMode::Show,
        })
    }
}
```

- [ ] **Step 7: Export parser**

In `src/agent/mod.rs`, add `CodexHookParser` to the parser exports:

```rust
pub use parser::{ClaudeHookParser, CodexHookParser, HookParser, OpenCodeHookParser};
```

- [ ] **Step 8: Run focused Codex parser tests**

Run: `cargo test test_codex_hook_ -v`

Expected: PASS.

- [ ] **Step 9: Run all parser tests**

Run: `cargo test agent::parser -v`

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add src/agent/parser.rs src/agent/mod.rs
git commit -m "feat: parse codex hook events"
```

---

### Task 5: Add `codex-hook` subcommand

**Files:**
- Create: `src/codex_hook.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create Codex hook runner**

Create `src/codex_hook.rs`:

```rust
use std::io::Read;

use anyhow::{Context, Result};

use crate::agent::parser::DisplayMode;
use crate::agent::state::AgentView;
use crate::agent::{CodexHookParser, HookParser};
use crate::ipc;

/// Run the Codex hook subcommand: read stdin JSON, parse via CodexHookParser, send state via IPC.
pub async fn run() -> Result<()> {
    let pane_id = std::env::var("TMUX_PANE")
        .context("TMUX_PANE environment variable not set (not running inside tmux?)")?;

    let mut stdin_buf = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin_buf)
        .context("Failed to read stdin")?;

    let parser = CodexHookParser;
    let result = parser.parse(pane_id, &stdin_buf)?;

    if result.display == DisplayMode::Suppress {
        return Ok(());
    }

    eprintln!(
        "[chikuwa codex-hook] event: {} agent_id: {:?}",
        result.state.event_label(),
        result.state.agent_id()
    );

    ipc::broadcast_state(&result.state).await?;

    if let Err(e) = crate::persist::append_agent_state(&result.state) {
        eprintln!("Warning: failed to persist agent state: {}", e);
    }

    Ok(())
}
```

- [ ] **Step 2: Wire module and CLI**

Update `src/main.rs`:

```rust
mod codex_hook;
```

Add enum variant:

```rust
/// Codex hook mode: update agent state from Codex CLI hooks (reads event from stdin JSON)
CodexHook,
```

Add match arm:

```rust
Some(Commands::CodexHook) => {
    codex_hook::run().await?;
}
```

- [ ] **Step 3: Run help command**

Run: `cargo run -- --help`

Expected: output includes `codex-hook`.

- [ ] **Step 4: Run build**

Run: `cargo build`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/codex_hook.rs src/main.rs
git commit -m "feat: add codex hook command"
```

---

### Task 6: Route Codex subagents in app state

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add failing app test for Codex subagent routing predicate**

Add a small helper near app logic:

```rust
fn is_subagent_state(state: &AgentState) -> bool {
    state.agent_id().is_some()
        && matches!(
            state.source(),
            crate::agent::state::AgentSource::Claude | crate::agent::state::AgentSource::Codex
        )
}
```

Add this test in `src/app.rs` test module:

```rust
#[test]
fn test_codex_agent_id_routes_as_subagent() {
    let state = AgentState::new(
        "%1".to_string(),
        AgentData::Codex(crate::agent::codex_state::CodexState {
            session_id: Some("sess-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            status: AgentStatus::Running,
            hook_event_name: "SubagentStart".to_string(),
            event_emoji: "🤖".to_string(),
            tool_name: None,
            tool_detail: None,
            active_tools: Vec::new(),
            failure_detail: None,
            turn_id: Some("turn-1".to_string()),
            permission_mode: None,
            model: None,
            cwd: None,
            agent_type: Some("code-review".to_string()),
            transcript_path: None,
        }),
    );

    assert!(is_subagent_state(&state));
}
```

- [ ] **Step 2: Replace inline condition in run_app**

Find the `AppEvent::AgentStateUpdate(state)` branch and replace:

```rust
if state.agent_id().is_some()
    && state.source() == crate::agent::state::AgentSource::Claude
{
```

with:

```rust
if is_subagent_state(&state) {
```

- [ ] **Step 3: Run app tests**

Run: `cargo test app:: -v`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: route codex subagent events"
```

---

### Task 7: Document Codex hooks setup

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update subcommands and data flow**

Add `codex-hook` to the README command table and data flow diagram:

```markdown
| Codex Hook | `chikuwa codex-hook` | Called from Codex CLI hooks; reads event JSON from stdin to update agent status via IPC |
```

```text
Codex CLI ──(hooks)──→ chikuwa codex-hook ──(IPC)──→ chikuwa (TUI)
```

- [ ] **Step 2: Add Codex CLI Hooks Setup section**

Add after the Claude Code hook setup:

````markdown
### Codex CLI Hooks Setup

Configure Codex CLI hooks in `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "startup|resume|clear|compact",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "SubagentStart": [{
      "matcher": ".*",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "PreToolUse": [{
      "matcher": ".*",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "PermissionRequest": [{
      "matcher": ".*",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "PostToolUse": [{
      "matcher": ".*",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "PreCompact": [{
      "matcher": "manual|auto",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "PostCompact": [{
      "matcher": "manual|auto",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "UserPromptSubmit": [{
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "SubagentStop": [{
      "matcher": ".*",
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }],
    "Stop": [{
      "hooks": [{"type": "command", "command": "chikuwa codex-hook"}]
    }]
  }
}
```

Codex requires non-managed command hooks to be reviewed and trusted before they run. Use `/hooks` inside Codex CLI to review/trust the configured chikuwa hook commands.
````

- [ ] **Step 3: Add Codex event mapping table**

```markdown
Codex events map to AgentStatus:

| Codex Event | AgentStatus |
|---|---|
| `SessionStart` | `Started` |
| `SubagentStart`, `PreToolUse`, `PostToolUse`, `PreCompact`, `PostCompact`, `UserPromptSubmit` | `Running` |
| `PermissionRequest` | `Permission` |
| `SubagentStop` | `Ended` |
| `Stop` | `Waiting` |
```

- [ ] **Step 4: Run README grep sanity check**

Run: `rg "codex-hook|Codex CLI Hooks|Codex events" README.md -n`

Expected: finds all added sections.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: add codex hook setup"
```

---

### Task 8: Final verification

**Files:**
- Run only

- [ ] **Step 1: Format check**

Run: `cargo fmt --check`

Expected: PASS.

- [ ] **Step 2: Lint check**

Run: `cargo clippy -- -D warnings`

Expected: PASS, no warnings.

- [ ] **Step 3: Full tests**

Run: `cargo test`

Expected: PASS, all tests pass.

- [ ] **Step 4: Help smoke test**

Run: `cargo run -- --help`

Expected: output includes `codex-hook`.

- [ ] **Step 5: Manual parser smoke test**

Run:

```bash
printf '%s\n' '{"hook_event_name":"SessionStart","session_id":"sess-1","cwd":"/repo","model":"o3","source":"startup"}' \
  | TMUX_PANE=%1 cargo run -- codex-hook
```

Expected: command exits `0`; if no TUI socket is running, IPC broadcast may be a no-op or log a warning depending on existing IPC behavior, but JSON parsing must succeed.

- [ ] **Step 6: Commit final formatting/doc cleanup if needed**

```bash
git add -A
git commit -m "chore: finalize codex hooks integration"
```

---

## Self-Review Notes

- Spec coverage: plan covers all 10 Codex hook events, Codex state, parser, CLI, subagent routing, docs, and verification.
- Placeholder scan: no `TBD`, `TODO`, or unspecified “add tests” steps remain.
- Type consistency: `ToolKey::Codex { tool_use_id }`, `AgentSource::Codex`, `AgentData::Codex`, `CodexState`, and `CodexHookParser` names are consistent across tasks.
- Scope check: one cohesive subsystem; no independent feature needs splitting.
