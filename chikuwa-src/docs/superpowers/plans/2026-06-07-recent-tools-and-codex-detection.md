# Recent Tools And Codex Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show each agent's most recent completed tool calls above currently active tools, and detect `codex` panes as agent panes even before hook state arrives.

**Architecture:** Keep `active_tools` semantics unchanged: it only means tools currently in flight. Add `recent_tools` to the shared `AgentView` surface and to each source-specific state (`ClaudeState`, `CodexState`, `OpenCodeState`) plus subagents. Source-specific merge logic moves tools from active to a bounded recent queue on completion. Tree and Office views render recent tools first in oldest-to-newest order, then active tools at the bottom.

**Tech Stack:** Rust, serde, ratatui, existing chikuwa TUI architecture

---

## Requirements

- Recent completed tools are grouped above active tools.
- Recent ordering is oldest at top, newest lower down.
- Active tools remain at the bottom and keep spinner-style active rendering.
- Recent count is a centralized constant, default `3`.
- `Stop` / waiting clears active tools but preserves recent tools.
- `SessionEnd` / ended removes the agent state as it does today.
- Subagents support the same recent/active tool grouping.
- Codex process detection recognizes `pane_current_command == "codex"` like Claude.
- No new colors; use existing palette and static symbols for completed/recent tools.

## File Inventory

| Action | File | Responsibility |
|---|---|---|
| Modify | `src/agent/state.rs` | Add `RECENT_TOOLS_LIMIT`, `recent_tools()` trait method, bounded recent helper |
| Modify | `src/agent/claude.rs` | Add `recent_tools` field and merge completed/failed tools into recent history |
| Modify | `src/agent/codex_state.rs` | Add `recent_tools` field and merge completed tools into recent history |
| Modify | `src/agent/opencode_state.rs` | Add `recent_tools` field and merge completed/error tools into recent history |
| Modify | `src/agent/parser.rs` | Populate `recent_tools: Vec::new()` in parser-created states and tests |
| Modify | `src/agent/subagent.rs` | Add `recent_tools` to `SubagentInfo` |
| Modify | `src/app.rs` | Maintain subagent recent tool history and update test constructors |
| Modify | `src/ui/tree.rs` | Render recent tools above active tools; detect `codex` command |
| Modify | `src/ui/office.rs` | Render recent tools above active tools in room and subagent blocks |
| Modify | `README.md` | Mention recent tools and Codex process detection |

---

### Task 1: Add Recent Tool Surface To Shared State

**Files:**
- Modify: `src/agent/state.rs`

- [ ] **Step 1: Write failing tests for bounded recent helper**

Add these tests inside `#[cfg(test)] mod tests` in `src/agent/state.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test push_recent_tool -v`

Expected: compile fails because `RECENT_TOOLS_LIMIT` and `push_recent_tool` do not exist.

- [ ] **Step 3: Add constant, trait method, and helper**

In `src/agent/state.rs`, add after `ActiveTool`:

```rust
pub const RECENT_TOOLS_LIMIT: usize = 3;

pub fn push_recent_tool(recent_tools: &mut Vec<ActiveTool>, tool: ActiveTool) {
    if let Some(pos) = recent_tools.iter().position(|existing| existing.key == tool.key) {
        recent_tools.remove(pos);
    }
    recent_tools.push(tool);
    let overflow = recent_tools.len().saturating_sub(RECENT_TOOLS_LIMIT);
    if overflow > 0 {
        recent_tools.drain(0..overflow);
    }
}
```

Update the `AgentView` trait:

```rust
fn recent_tools(&self) -> &[ActiveTool];
```

Update the `impl AgentView for AgentState`:

```rust
fn recent_tools(&self) -> &[ActiveTool] {
    match &self.data {
        AgentData::Claude(c) => &c.recent_tools,
        AgentData::OpenCode(o) => &o.recent_tools,
        AgentData::Codex(c) => &c.recent_tools,
    }
}
```

- [ ] **Step 4: Run focused tests**

Run: `cargo test push_recent_tool -v`

Expected: tests pass, but other code may not compile yet because state structs do not have `recent_tools`. Continue to Task 2 if compile fails for missing fields.

---

### Task 2: Add `recent_tools` To ClaudeState

**Files:**
- Modify: `src/agent/claude.rs`
- Modify: `src/agent/parser.rs`
- Modify: all tests/constructors that create `ClaudeState`

- [ ] **Step 1: Write failing Claude merge tests**

Add to `src/agent/claude.rs` tests:

```rust
#[test]
fn test_post_tool_use_moves_completed_tool_to_recent() {
    let mut existing = make_state("PreToolUse", AgentStatus::Running);
    existing.active_tools.push(tool_with_id("Bash", "toolu_01"));

    let mut incoming = make_state("PostToolUse", AgentStatus::Running);
    incoming.active_tools.push(tool_with_id("Bash", "toolu_01"));

    let merged = ClaudeState::merge(incoming, &existing);

    assert!(merged.active_tools.is_empty());
    assert_eq!(merged.recent_tools.len(), 1);
    assert_eq!(merged.recent_tools[0].name, "Bash");
}

#[test]
fn test_post_tool_use_failure_moves_failed_tool_to_recent() {
    let mut existing = make_state("PreToolUse", AgentStatus::Running);
    existing.active_tools.push(tool_with_id("Bash", "toolu_01"));

    let mut incoming = make_state("PostToolUseFailure", AgentStatus::Running);
    incoming.active_tools.push(tool_with_id("Bash", "toolu_01"));
    incoming.failure_detail = Some("exit code 1".to_string());

    let merged = ClaudeState::merge(incoming, &existing);

    assert!(merged.active_tools.is_empty());
    assert_eq!(merged.recent_tools.len(), 1);
    assert_eq!(merged.recent_tools[0].failure_detail.as_deref(), Some("exit code 1"));
}

#[test]
fn test_stop_preserves_recent_tools() {
    let mut existing = make_state("PreToolUse", AgentStatus::Running);
    existing.recent_tools.push(tool_with_id("Bash", "toolu_01"));
    existing.active_tools.push(tool_with_id("Read", "toolu_02"));

    let incoming = make_state("Stop", AgentStatus::Waiting);
    let merged = ClaudeState::merge(incoming, &existing);

    assert!(merged.active_tools.is_empty());
    assert_eq!(merged.recent_tools.len(), 1);
    assert_eq!(merged.recent_tools[0].name, "Bash");
}
```

- [ ] **Step 2: Run Claude tests to verify RED**

Run: `cargo test agent::claude::tests::test_post_tool_use_moves_completed_tool_to_recent -v`

Expected: compile fails because `ClaudeState` has no `recent_tools` field.

- [ ] **Step 3: Add field to ClaudeState**

In `src/agent/claude.rs`, add after `active_tools`:

```rust
/// Recently completed tool calls, oldest to newest.
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub recent_tools: Vec<ActiveTool>,
```

- [ ] **Step 4: Update ClaudeState merge imports and logic**

Change imports at top:

```rust
use super::state::{push_recent_tool, ActiveTool, AgentStatus};
```

Inside `ClaudeState::merge`, create recent history before building `merged`:

```rust
let mut recent_tools = existing.recent_tools.clone();
```

Inside the `"PostToolUse" | "PostToolUseFailure"` branch, after finding/removing a matching active tool, push the completed tool to recent:

```rust
let completed = if let Some(pos) = pos {
    tools.remove(pos)
} else {
    removing.clone()
};
let mut completed = completed;
if event == "PostToolUseFailure" {
    completed.failure_detail = incoming.failure_detail.clone();
}
push_recent_tool(&mut recent_tools, completed);
```

For non-running statuses, keep existing behavior for active tools but do not clear recent tools:

```rust
let active_tools = if incoming.status == AgentStatus::Running { /* existing match */ } else { Vec::new() };
```

After `merged.active_tools = active_tools;`, add:

```rust
merged.recent_tools = recent_tools;
```

If `is_silent`, preserve visual state exactly as now, but keep the new `merged.recent_tools` value.

- [ ] **Step 5: Update ClaudeState constructors**

Every `ClaudeState { ... }` literal in `src/agent/parser.rs`, `src/app.rs`, `src/ui/tree.rs`, `src/ui/office.rs`, and `src/agent/state.rs` must include:

```rust
recent_tools: Vec::new(),
```

In `src/agent/claude.rs` test helper `make_state`, include:

```rust
recent_tools: Vec::new(),
```

- [ ] **Step 6: Run Claude tests**

Run: `cargo test agent::claude -v`

Expected: all Claude tests pass.

---

### Task 3: Add `recent_tools` To CodexState

**Files:**
- Modify: `src/agent/codex_state.rs`
- Modify: `src/agent/parser.rs`
- Modify: all tests/constructors that create `CodexState`

- [ ] **Step 1: Write failing Codex merge tests**

Add to `src/agent/codex_state.rs` tests:

```rust
#[test]
fn test_post_tool_use_moves_completed_tool_to_recent() {
    let existing = make_state(
        "PreToolUse",
        AgentStatus::Running,
        Some("Bash"),
        Some("call-1"),
    );
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
}

#[test]
fn test_stop_preserves_recent_tools() {
    let mut existing = make_state(
        "PreToolUse",
        AgentStatus::Running,
        Some("Bash"),
        Some("call-1"),
    );
    existing.recent_tools.push(ActiveTool {
        key: ToolKey::Codex {
            tool_use_id: "call-old".to_string(),
        },
        name: "Read".to_string(),
        detail: None,
        failure_detail: None,
    });
    let incoming = make_state("Stop", AgentStatus::Waiting, None, None);

    let merged = CodexState::merge(incoming, &existing);

    assert!(merged.active_tools.is_empty());
    assert_eq!(merged.recent_tools.len(), 1);
    assert_eq!(merged.recent_tools[0].name, "Read");
}
```

- [ ] **Step 2: Run Codex tests to verify RED**

Run: `cargo test agent::codex_state::tests::test_post_tool_use_moves_completed_tool_to_recent -v`

Expected: compile fails because `CodexState` has no `recent_tools` field.

- [ ] **Step 3: Add field and constructor default**

In `src/agent/codex_state.rs`, add after `active_tools`:

```rust
/// Recently completed tool calls, oldest to newest.
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub recent_tools: Vec<ActiveTool>,
```

In `CodexState::new`, add:

```rust
recent_tools: Vec::new(),
```

- [ ] **Step 4: Update Codex merge logic**

Change imports:

```rust
use super::state::{push_recent_tool, ActiveTool, AgentStatus};
```

Inside `CodexState::merge`, mirror the Claude approach:

```rust
let mut recent_tools = existing.recent_tools.clone();
```

Inside `"PostToolUse"`, remove active tool and push it to recent:

```rust
let completed = if let Some(pos) = pos {
    tools.remove(pos)
} else {
    removing.clone()
};
push_recent_tool(&mut recent_tools, completed);
```

After assigning `merged.active_tools`, add:

```rust
merged.recent_tools = recent_tools;
```

If `is_silent`, preserve visual state but keep updated `recent_tools`.

- [ ] **Step 5: Update CodexState literals**

Every `CodexState { ... }` literal in `src/agent/parser.rs`, `src/app.rs`, and `src/agent/state.rs` tests must include:

```rust
recent_tools: Vec::new(),
```

- [ ] **Step 6: Run Codex tests**

Run: `cargo test agent::codex_state -v`

Expected: all Codex state tests pass.

---

### Task 4: Add `recent_tools` To OpenCodeState

**Files:**
- Modify: `src/agent/opencode_state.rs`
- Modify: `src/agent/parser.rs`

- [ ] **Step 1: Write failing OpenCode tests**

Add to `src/agent/opencode_state.rs` tests:

```rust
#[test]
fn test_tool_completed_moves_tool_to_recent() {
    let mut existing = make_state("tool.execute");
    existing.active_tools.push(opencode_tool("bash", Some("ls")));

    let mut incoming = make_state("tool.completed");
    incoming.active_tools.push(opencode_tool("bash", Some("ls")));

    let merged = OpenCodeState::merge(incoming, &existing);

    assert!(merged.active_tools.is_empty());
    assert_eq!(merged.recent_tools.len(), 1);
    assert_eq!(merged.recent_tools[0].name, "bash");
}

#[test]
fn test_tool_error_moves_failed_tool_to_recent() {
    let mut existing = make_state("tool.execute");
    existing.active_tools.push(opencode_tool("bash", Some("ls")));

    let mut incoming = make_state("tool.error");
    incoming.active_tools.push(opencode_tool("bash", Some("ls")));

    let merged = OpenCodeState::merge(incoming, &existing);

    assert!(merged.active_tools.is_empty());
    assert_eq!(merged.recent_tools.len(), 1);
    assert_eq!(merged.recent_tools[0].failure_detail.as_deref(), Some("tool.error"));
}

#[test]
fn test_session_idle_preserves_recent_tools() {
    let mut existing = make_state("tool.execute");
    existing.recent_tools.push(opencode_tool("read", Some("src/main.rs")));
    existing.active_tools.push(opencode_tool("bash", Some("ls")));

    let incoming = make_state("session.idle");
    let merged = OpenCodeState::merge(incoming, &existing);

    assert!(merged.active_tools.is_empty());
    assert_eq!(merged.recent_tools.len(), 1);
    assert_eq!(merged.recent_tools[0].name, "read");
}
```

- [ ] **Step 2: Run OpenCode tests to verify RED**

Run: `cargo test agent::opencode_state::tests::test_tool_completed_moves_tool_to_recent -v`

Expected: compile fails because `OpenCodeState` has no `recent_tools` field.

- [ ] **Step 3: Add field and merge logic**

In `src/agent/opencode_state.rs`, add after `active_tools`:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub recent_tools: Vec<ActiveTool>,
```

Change imports:

```rust
use super::state::{push_recent_tool, ActiveTool, AgentStatus};
```

Inside `OpenCodeState::merge`, start with:

```rust
let mut recent_tools = existing.recent_tools.clone();
```

In `"tool.completed" | "tool.error"`, when removing a tool, push it to recent:

```rust
let completed = if let Some(pos) = pos {
    tools.remove(pos)
} else {
    removing.clone()
};
let mut completed = completed;
if event == "tool.error" {
    completed.failure_detail = Some("tool.error".to_string());
}
push_recent_tool(&mut recent_tools, completed);
```

After `merged.active_tools = active_tools;`, add:

```rust
merged.recent_tools = recent_tools;
```

Session idle/deleted still clears active tools as today; keep recent tools unless status is `Ended`.

- [ ] **Step 4: Update OpenCodeState constructors**

Every `OpenCodeState { ... }` literal in `src/agent/parser.rs` and `src/agent/opencode_state.rs` tests must include:

```rust
recent_tools: Vec::new(),
```

- [ ] **Step 5: Run OpenCode tests**

Run: `cargo test agent::opencode_state -v`

Expected: all OpenCode tests pass.

---

### Task 5: Add Recent Tools To SubagentInfo

**Files:**
- Modify: `src/agent/subagent.rs`
- Modify: `src/app.rs`
- Modify: tests that create `SubagentInfo`

- [ ] **Step 1: Write failing subagent test**

Add to `src/app.rs` tests:

```rust
#[test]
fn test_subagent_post_tool_use_moves_tool_to_recent() {
    let mut app = App::new();
    let pane_id = "%1".to_string();
    let agent_id = "agent-1".to_string();

    let start = AgentState::new(
        pane_id.clone(),
        AgentData::Claude(ClaudeState {
            session_id: Some("sess-1".to_string()),
            agent_id: Some(agent_id.clone()),
            status: AgentStatus::Running,
            hook_event_name: "PreToolUse".to_string(),
            event_emoji: "🔧".to_string(),
            tool_name: Some("Bash".to_string()),
            tool_detail: Some("ls".to_string()),
            active_tools: vec![crate::agent::state::ActiveTool {
                key: crate::agent::state::ToolKey::Claude {
                    tool_use_id: "toolu_1".to_string(),
                },
                name: "Bash".to_string(),
                detail: Some("ls".to_string()),
                failure_detail: None,
            }],
            recent_tools: Vec::new(),
            failure_detail: None,
        }),
    );

    let stop = AgentState::new(
        pane_id.clone(),
        AgentData::Claude(ClaudeState {
            session_id: Some("sess-1".to_string()),
            agent_id: Some(agent_id.clone()),
            status: AgentStatus::Running,
            hook_event_name: "PostToolUse".to_string(),
            event_emoji: "✅".to_string(),
            tool_name: Some("Bash".to_string()),
            tool_detail: Some("ls".to_string()),
            active_tools: vec![crate::agent::state::ActiveTool {
                key: crate::agent::state::ToolKey::Claude {
                    tool_use_id: "toolu_1".to_string(),
                },
                name: "Bash".to_string(),
                detail: Some("ls".to_string()),
                failure_detail: None,
            }],
            recent_tools: Vec::new(),
            failure_detail: None,
        }),
    );

    app.merge_subagent_state(pane_id.clone(), agent_id.clone(), start);
    app.merge_subagent_state(pane_id.clone(), agent_id.clone(), stop);

    let info = app.subagent_states.get(&(pane_id, agent_id)).unwrap();
    assert!(info.tools.is_empty());
    assert_eq!(info.recent_tools.len(), 1);
    assert_eq!(info.recent_tools[0].name, "Bash");
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test test_subagent_post_tool_use_moves_tool_to_recent -v`

Expected: compile fails because `SubagentInfo` has no `recent_tools` field.

- [ ] **Step 3: Add field to SubagentInfo**

In `src/agent/subagent.rs`, add after `tools`:

```rust
/// Recently completed tools, oldest to newest
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub recent_tools: Vec<ActiveTool>,
```

In `SubagentInfo::new`, add:

```rust
recent_tools: Vec::new(),
```

- [ ] **Step 4: Update app subagent merge logic**

In `src/app.rs`, update imports in `merge_subagent_state` scope to use helper:

```rust
use crate::agent::state::{push_recent_tool, AgentStatus};
```

In the `PostToolUse` / `PostToolUseFailure` branch for occupied subagents, replace `info.tools.remove(pos);` with:

```rust
let completed = if let Some(pos) = info
    .tools
    .iter()
    .position(|t| t.key == removing.key)
    .or_else(|| info.tools.iter().position(|t| t.name == removing.name))
{
    info.tools.remove(pos)
} else {
    removing.clone()
};
let mut completed = completed;
if event == "PostToolUseFailure" {
    completed.failure_detail = state.failure_detail().map(str::to_string);
}
push_recent_tool(&mut info.recent_tools, completed);
```

When creating `ended_info`, include:

```rust
recent_tools: Vec::new(),
```

- [ ] **Step 5: Update SubagentInfo literals and tests**

Any `SubagentInfo { ... }` literal must include:

```rust
recent_tools: Vec::new(),
```

- [ ] **Step 6: Run app/subagent tests**

Run: `cargo test app::tests::test_subagent_post_tool_use_moves_tool_to_recent agent::subagent -v`

Expected: tests pass.

---

### Task 6: Render Recent Tools In Tree View And Detect Codex

**Files:**
- Modify: `src/ui/tree.rs`

- [ ] **Step 1: Write failing tree tests**

Add to `src/ui/tree.rs` tests:

```rust
#[test]
fn test_is_agent_command_codex() {
    assert!(is_agent_command("codex", None));
    assert!(is_agent_command("codex", Some("anything")));
}

#[test]
fn test_agent_status_visual_rows_includes_recent_and_active() {
    let mut state = make_agent_state("%0", AgentStatus::Running);
    if let AgentData::Claude(ref mut claude) = state.data {
        claude.recent_tools.push(ActiveTool {
            key: ToolKey::Claude { tool_use_id: "old".to_string() },
            name: "Read".to_string(),
            detail: Some("src/lib.rs".to_string()),
            failure_detail: None,
        });
        claude.active_tools.push(ActiveTool {
            key: ToolKey::Claude { tool_use_id: "active".to_string() },
            name: "Bash".to_string(),
            detail: Some("cargo test".to_string()),
            failure_detail: None,
        });
    }
    let item = TreeItem::Pane {
        pane: make_pane("%0", "claude", Some(state)),
        session_name: "main".to_string(),
        window_index: 0,
        session_toplevel: None,
        session_attached: true,
    };

    assert_eq!(agent_status_visual_rows(&item), 3);
}
```

- [ ] **Step 2: Run tree tests to verify RED**

Run: `cargo test test_is_agent_command_codex test_agent_status_visual_rows_includes_recent_and_active -v`

Expected: `test_is_agent_command_codex` fails and/or compile fails until recent support is wired.

- [ ] **Step 3: Detect Codex command**

Update `is_agent_command`:

```rust
if command == "claude" || command == "codex" {
    return true;
}
```

Update comments from “Claude Code or OpenCode” to “Claude Code, Codex, or OpenCode”.

- [ ] **Step 4: Update tree row count**

Change `agent_status_visual_rows` to count recent plus active:

```rust
let recent_count = agent.recent_tools().len().min(RECENT_TOOLS_LIMIT);
let active_count = agent.active_tools().len().min(MAX_VISIBLE_TOOLS);
1 + recent_count
    + active_count
    + if agent.failure_detail().is_some() { 1 } else { 0 }
```

Add import at top of `src/ui/tree.rs` if needed:

```rust
use crate::agent::state::RECENT_TOOLS_LIMIT;
```

- [ ] **Step 5: Render recent before active**

In `render_bordered_agent_status_sub_lines`, add before active tool loop:

```rust
let recent_tools = agent.recent_tools();
let visible_recent = if recent_tools.len() > RECENT_TOOLS_LIMIT {
    &recent_tools[recent_tools.len() - RECENT_TOOLS_LIMIT..]
} else {
    recent_tools
};
for tool in visible_recent {
    let marker = if tool.failure_detail.is_some() { "✕" } else { "✓" };
    let tool_text = match &tool.detail {
        Some(detail) => {
            let display_detail = shorten_tool_detail(&tool.name, detail, toplevel);
            format!("{} {}: {}", marker, tool.name, display_detail)
        }
        None => format!("{} {}", marker, tool.name),
    };
    let mut tool_spans = vec![
        Span::styled(format!("{}  ", prefix), Style::default().fg(theme::COLOR_PURPLE)),
        Span::styled(tool_text, dim_style),
    ];
    truncate_spans(&mut tool_spans, content_width);
    result.push(wrap_bordered_line(tool_spans, content_width, selected, border_style));
}
```

Keep existing active tool loop after this recent loop so active stays at the bottom.

- [ ] **Step 6: Run tree tests**

Run: `cargo test ui::tree -v`

Expected: all tree tests pass.

---

### Task 7: Render Recent Tools In Office View

**Files:**
- Modify: `src/ui/office.rs`

- [ ] **Step 1: Write failing office test**

Add to `src/ui/office.rs` tests:

```rust
#[test]
fn test_office_renders_recent_before_active() {
    let mut state = make_agent_state("%0", AgentStatus::Running);
    if let AgentData::Claude(ref mut claude) = state.data {
        claude.recent_tools.push(ActiveTool {
            key: ToolKey::Claude { tool_use_id: "old".to_string() },
            name: "Read".to_string(),
            detail: Some("src/lib.rs".to_string()),
            failure_detail: None,
        });
        claude.active_tools.push(ActiveTool {
            key: ToolKey::Claude { tool_use_id: "active".to_string() },
            name: "Bash".to_string(),
            detail: Some("cargo test".to_string()),
            failure_detail: None,
        });
    }

    let lines = render_agent_room(&state, &[], 0, 80, false, false, 0);
    let joined = lines
        .iter()
        .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");

    let recent_pos = joined.find("✓ Read").unwrap();
    let active_pos = joined.find("cargo test").unwrap();
    assert!(recent_pos < active_pos);
}
```

- [ ] **Step 2: Run office test to verify RED**

Run: `cargo test test_office_renders_recent_before_active -v`

Expected: fails because recent tools are not rendered.

- [ ] **Step 3: Add helper to render tool line**

In `src/ui/office.rs`, add near `render_agent_room`:

```rust
fn render_tool_line(
    label: &str,
    content_width: usize,
    border_style: Style,
    bg_color: Option<Color>,
    is_selected: bool,
) -> Line<'static> {
    let dim_style = Style::default().fg(theme::COLOR_DIM);
    let tool_text = truncate_to_width(label, content_width.saturating_sub(4));
    let mut spans = vec![
        Span::styled("│ ".to_string(), border_style),
        Span::styled(format!(" {}", tool_text), dim_style),
    ];
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let pad = content_width.saturating_sub(used + 1);
    spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
    spans.push(Span::styled("│".to_string(), border_style));
    apply_bg(&mut spans, bg_color, is_selected);
    Line::from(spans)
}
```

- [ ] **Step 4: Replace active loop and add recent loop**

In `render_agent_room`, replace the active tool rendering block with:

```rust
for tool in agent.recent_tools() {
    let marker = if tool.failure_detail.is_some() { "✕" } else { "✓" };
    let label = match tool.detail.as_deref() {
        Some(detail) => format!("{} {}: {}", marker, tool.name, detail),
        None => format!("{} {}", marker, tool.name),
    };
    lines.push(render_tool_line(&label, content_width, border_style, bg_color, is_selected));
}

for tool in agent.active_tools() {
    let label = tool.detail.as_deref().unwrap_or(&tool.name);
    lines.push(render_tool_line(label, content_width, border_style, bg_color, is_selected));
}
```

- [ ] **Step 5: Render subagent recent before active**

In the subagent rendering loop, before `for tool in &sub.tools`, add:

```rust
for tool in &sub.recent_tools {
    let marker = if tool.failure_detail.is_some() { "✕" } else { "✓" };
    let detail = tool.detail.as_deref().unwrap_or(&tool.name);
    let tool_label = format!("{} {}: {}", marker, tool.name, detail);
    let tool_text = truncate_to_width(&tool_label, content_width.saturating_sub(prefix_width + 3));
    let mut spans = vec![
        Span::styled("│ ".to_string(), border_style),
        Span::styled(prefix.clone(), dim_style),
        Span::styled(format!("{}", tool_text), dim_style),
    ];
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let pad = content_width.saturating_sub(used + 1);
    spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
    spans.push(Span::styled("│".to_string(), border_style));
    apply_bg(&mut spans, bg_color, is_selected);
    lines.push(Line::from(spans));
}
```

Keep existing `for tool in &sub.tools` after recent loop so active subagent tools stay lower.

- [ ] **Step 6: Run office tests**

Run: `cargo test ui::office -v`

Expected: all office tests pass.

---

### Task 8: Update Persistence Compatibility And Parser Constructors

**Files:**
- Modify: `src/persist.rs` if tests need explicit old JSON coverage
- Modify: `src/agent/parser.rs`
- Modify: any compile-failing constructors from previous tasks

- [ ] **Step 1: Add old JSON compatibility test**

Add to `src/agent/state.rs` tests:

```rust
#[test]
fn test_old_agent_state_json_without_recent_tools_deserializes() {
    let json = r#"{"tmux_pane":"%0","updated_at":100,"data":{"type":"claude","status":"running","hook_event_name":"PreToolUse","event_emoji":"🔧","active_tools":[]}}"#;
    let state: AgentState = serde_json::from_str(json).unwrap();

    assert!(state.recent_tools().is_empty());
}
```

- [ ] **Step 2: Run compatibility test**

Run: `cargo test test_old_agent_state_json_without_recent_tools_deserializes -v`

Expected: PASS once every source state has `#[serde(default)]` on `recent_tools`.

- [ ] **Step 3: Fix constructor compile errors**

Run: `cargo test --no-run`

Expected: if compile errors mention missing `recent_tools`, add `recent_tools: Vec::new(),` to that literal.

- [ ] **Step 4: Run targeted agent tests**

Run: `cargo test agent:: -v`

Expected: all agent tests pass.

---

### Task 9: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document recent tools and Codex detection**

Update the Features section bullets:

```markdown
- **Active and recent tool display** — Shows currently running tools at the bottom of each agent block and the last 3 completed tools above them
- **Agent process detection** — Recognizes Claude Code, Codex CLI, and OpenCode panes even before hook state arrives
```

Update How It Works text to mention Codex panes:

```markdown
Agent panes are detected from hook state first, and by process name as a fallback (`claude`, `codex`, or `node` windows containing `opencode`).
```

- [ ] **Step 2: Run README grep check**

Run: `rg "recent tool|Codex CLI|codex" README.md -n`

Expected: new documentation appears.

---

### Task 10: Final Verification

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

- [ ] **Step 4: Manual UI smoke check**

Run: `cargo run`

Expected: TUI starts. In a tmux pane running an agent, completed tools stay visible above active tools, and active tools remain at the bottom.

- [ ] **Step 5: Commit final work**

```bash
git add README.md src/agent/state.rs src/agent/claude.rs src/agent/codex_state.rs src/agent/opencode_state.rs src/agent/parser.rs src/agent/subagent.rs src/app.rs src/ui/tree.rs src/ui/office.rs
git commit -m "feat: show recent agent tools"
```

---

## Self-Review Notes

- Requirements coverage: recent completed tools, old-to-new ordering, active-at-bottom ordering, centralized limit, subagents, Codex process detection, docs, and verification are all covered.
- Scope check: this is one cohesive UI/state feature; no independent subsystem split is needed.
- Type consistency: `recent_tools`, `RECENT_TOOLS_LIMIT`, and `push_recent_tool` names are consistent across tasks.
- Placeholder scan: no placeholder task remains; all implementation steps include concrete code snippets or exact commands.
