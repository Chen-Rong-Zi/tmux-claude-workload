# Agent Office View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an "Agent Office" view page, switchable with h/l keys, showing a humanized office layout where each agent is a "room" with its status and activities.

**Architecture:** Add a `ViewMode` enum (Tree/Office) to `App`. Handle h/l keys to switch views. Create `src/ui/office.rs` that renders the office layout — main agent room on top (large), subagent rooms in a row below (small). Only sessions with agents are shown. Permission-needing agents get a highlighted background color + 🔐 icon. The office view reuses existing `agent_states`, `subagent_states`, and `sessions` data from `App`.

**Tech Stack:** Rust, ratatui (Paragraph, Layout, Constraint), crossterm key events

---

### File Structure

| File | Responsibility |
|------|---------------|
| `src/ui/office.rs` | **Create** — Office view rendering: main agent room, subagent rooms, status summary |
| `src/ui/mod.rs` | **Modify** — Export `office` module |
| `src/event.rs` | **Modify** — Add `SwitchViewLeft`/`SwitchViewRight` actions for h/l keys |
| `src/app.rs` | **Modify** — Add `ViewMode` enum, `view_mode` field, handle view switching, delegate rendering based on view mode |
| `src/ui/theme.rs` | **Modify** — Add `COLOR_PERMISSION_BG` constant for permission room background |

---

### Task 1: Add ViewMode and h/l key bindings

**Files:**
- Modify: `src/event.rs`
- Modify: `src/app.rs`

- [ ] **Step 1: Add `SwitchViewLeft` and `SwitchViewRight` actions to `src/event.rs`**

Add to the `Action` enum:

```rust
#[derive(Debug, PartialEq)]
pub enum Action {
    Quit,
    Up,
    Down,
    Select,
    Top,
    Bottom,
    ToggleCollapse,
    SwitchViewLeft,
    SwitchViewRight,
    None,
}
```

Add key bindings in `handle_key`:

```rust
KeyCode::Char('h') | KeyCode::Left => Action::SwitchViewLeft,
KeyCode::Char('l') | KeyCode::Right => Action::SwitchViewRight,
```

Add test:

```rust
#[test]
fn test_switch_view_hl() {
    assert_eq!(handle_key(key(KeyCode::Char('h'))), Action::SwitchViewLeft);
    assert_eq!(handle_key(key(KeyCode::Char('l'))), Action::SwitchViewRight);
    assert_eq!(handle_key(key(KeyCode::Left)), Action::SwitchViewLeft);
    assert_eq!(handle_key(key(KeyCode::Right)), Action::SwitchViewRight);
}
```

- [ ] **Step 2: Add `ViewMode` enum and `view_mode` field to `src/app.rs`**

Add before the `App` struct:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Tree,
    Office,
}
```

Add field to `App`:

```rust
pub struct App {
    // ... existing fields ...
    view_mode: ViewMode,
    // ...
}
```

Initialize in `App::new()`:

```rust
view_mode: ViewMode::Tree,
```

- [ ] **Step 3: Handle `SwitchViewLeft`/`SwitchViewRight` in the event loop**

In `src/app.rs`, in the main event loop `match action { ... }`, add:

```rust
Action::SwitchViewLeft => {
    app.view_mode = match app.view_mode {
        ViewMode::Office => ViewMode::Tree,
        ViewMode::Tree => ViewMode::Office,
    };
}
Action::SwitchViewRight => {
    app.view_mode = match app.view_mode {
        ViewMode::Tree => ViewMode::Office,
        ViewMode::Office => ViewMode::Tree,
    };
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass including new `test_switch_view_hl`

- [ ] **Step 5: Commit**

```bash
git add src/event.rs src/app.rs
git commit -m "feat: add ViewMode enum and h/l key bindings for view switching"
```

---

### Task 2: Add permission background color to theme

**Files:**
- Modify: `src/ui/theme.rs`

- [ ] **Step 1: Add `COLOR_PERMISSION_BG` constant**

In `src/ui/theme.rs`, after the existing color constants:

```rust
pub const COLOR_PERMISSION_BG: Color = Color::Rgb(0x3a, 0x1a, 0x3a);
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add src/ui/theme.rs
git commit -m "feat: add COLOR_PERMISSION_BG for office view permission highlighting"
```

---

### Task 3: Create office view rendering module

**Files:**
- Create: `src/ui/office.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Create `src/ui/office.rs`**

```rust
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::agent::state::{AgentState, AgentStatus};
use crate::agent::{SubagentInfo, SubagentStatus};
use crate::tmux::types::TmuxSession;
use crate::ui::theme;

/// An agent entry for the office view (pre-computed from session data).
struct AgentEntry {
    pane_id: String,
    tmux_target: String,
    agent_state: AgentState,
    subagents: Vec<SubagentInfo>,
    completed_count: u32,
}

/// Collect all agent entries from sessions (only sessions with agents).
fn collect_agent_entries(
    sessions: &[TmuxSession],
    agent_states: &std::collections::HashMap<String, AgentState>,
    subagent_data: &std::collections::HashMap<String, (Vec<SubagentInfo>, u32)>,
) -> Vec<AgentEntry> {
    let mut entries = Vec::new();

    for session in sessions {
        for window in &session.windows {
            for pane in &window.panes {
                if let Some(ref state) = pane.agent_state {
                    let (subagents, completed) = subagent_data
                        .get(&pane.pane_id)
                        .cloned()
                        .unwrap_or_default();
                    entries.push(AgentEntry {
                        pane_id: pane.pane_id.clone(),
                        tmux_target: format!(
                            "{}:{}.{}",
                            session.session_name,
                            window.window_index,
                            pane.pane_index
                        ),
                        agent_state: state.clone(),
                        subagents,
                        completed_count: completed,
                    });
                }
            }
        }
    }

    entries
}

/// Format elapsed seconds as human-readable string.
fn format_duration(secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed = now.saturating_sub(secs);
    if elapsed < 60 {
        format!("{}s", elapsed)
    } else if elapsed < 3600 {
        format!("{}m {}s", elapsed / 60, elapsed % 60)
    } else {
        format!("{}h {}m", elapsed / 3600, (elapsed % 3600) / 60)
    }
}

/// Render the office view.
pub fn render(
    f: &mut Frame,
    area: Rect,
    sessions: &[TmuxSession],
    agent_states: &std::collections::HashMap<String, AgentState>,
    subagent_data: &std::collections::HashMap<String, (Vec<SubagentInfo>, u32)>,
    selected: usize,
    scroll_offset: usize,
    anim_frame: usize,
) {
    let entries = collect_agent_entries(sessions, agent_states, subagent_data);

    let lines = build_office_lines(&entries, area.width, selected, anim_frame);

    let visible_height = area.height as usize;
    let visible_lines: Vec<Line> = lines.into_iter().skip(scroll_offset).take(visible_height).collect();

    let paragraph = Paragraph::new(visible_lines);
    f.render_widget(paragraph, area);
}

fn build_office_lines(
    entries: &[AgentEntry],
    width: u16,
    selected: usize,
    anim_frame: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if entries.is_empty() {
        let msg = "  No active agents";
        lines.push(Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(Color::Rgb(0x7a, 0x7a, 0x7a)),
        )));
        return lines;
    }

    let content_width = (width as usize).saturating_sub(2); // padding

    for (idx, entry) in entries.iter().enumerate() {
        let is_selected = idx == selected;
        let is_permission = entry.agent_state.state == AgentStatus::Permission;

        // Main agent room
        let room_lines = render_agent_room(
            &entry.agent_state,
            &entry.subagents,
            entry.completed_count,
            content_width,
            is_selected,
            is_permission,
            anim_frame,
        );
        lines.extend(room_lines);

        // Separator between entries
        if idx < entries.len() - 1 {
            lines.push(Line::from(Span::styled(
                "  ···",
                Style::default().fg(theme::COLOR_PURPLE),
            )));
        }
    }

    // Status summary at bottom
    lines.push(Line::from(""));
    let mut running = 0u32;
    let mut waiting = 0u32;
    let mut permission = 0u32;
    let mut ended = 0u32;
    for entry in entries {
        match entry.agent_state.state {
            AgentStatus::Started | AgentStatus::Running => running += 1,
            AgentStatus::Waiting => waiting += 1,
            AgentStatus::Permission => permission += 1,
            AgentStatus::Ended => ended += 1,
        }
    }
    let mut summary_parts = Vec::new();
    if running > 0 {
        summary_parts.push(format!("🔧 {} running", running));
    }
    if waiting > 0 {
        summary_parts.push(format!("💤 {} waiting", waiting));
    }
    if permission > 0 {
        summary_parts.push(format!("🔐 {} needs you!", permission));
    }
    if ended > 0 {
        summary_parts.push(format!("🏁 {} ended", ended));
    }
    if !summary_parts.is_empty() {
        let summary = format!("  {}", summary_parts.join("   "));
        lines.push(Line::from(Span::styled(
            summary,
            Style::default().fg(theme::COLOR_LIGHT_PURPLE),
        )));
    }

    lines
}

fn render_agent_room(
    agent: &AgentState,
    subagents: &[SubagentInfo],
    completed_count: u32,
    content_width: usize,
    is_selected: bool,
    is_permission: bool,
    anim_frame: usize,
) -> Vec<Line<'static>> {
    let dim_style = Style::default().fg(Color::Rgb(0x7a, 0x7a, 0x7a));
    let name_style = if is_permission {
        Style::default().fg(theme::COLOR_WHITE).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::COLOR_LIGHT_PURPLE)
    };

    let bg_color = if is_permission {
        Some(theme::COLOR_PERMISSION_BG)
    } else {
        None
    };

    let status_label = match agent.state {
        AgentStatus::Started => "starting",
        AgentStatus::Running => "running",
        AgentStatus::Waiting => "waiting",
        AgentStatus::Permission => "needs input!",
        AgentStatus::Ended => "ended",
    };

    let status_emoji = agent.event_emoji.as_deref().unwrap_or(match agent.state {
        AgentStatus::Started => "🚀",
        AgentStatus::Running => "🔧",
        AgentStatus::Waiting => "💤",
        AgentStatus::Permission => "🔐",
        AgentStatus::Ended => "🏁",
    });

    let border_style = if is_permission {
        Style::default().fg(theme::COLOR_WHITE)
    } else {
        Style::default().fg(theme::COLOR_PURPLE)
    };

    let mut lines = Vec::new();

    // Room top border: ┌── 🤖 Agent Name ──── Status ──┐
    let name_text = format!("🤖 {}", agent.hook_event_name.as_deref().unwrap_or("Agent"));
    let status_text = format!("{} {}", status_emoji, status_label);
    let name_width = name_text.width();
    let status_width = status_text.width();
    let inner_width = content_width.saturating_sub(2); // ┌ and ┐
    let fill_len = inner_width.saturating_sub(name_width + status_width + 4); // 2 spaces + 2 ──
    let fill = "─".repeat(fill_len);

    let mut border_spans = vec![
        Span::styled("┌─ ".to_string(), border_style),
        Span::styled(name_text, name_style),
        Span::styled(format!(" ── {} ", fill), border_style),
        Span::styled(status_text, dim_style),
        Span::styled(" ─┐".to_string(), border_style),
    ];
    if let Some(bg) = bg_color {
        for span in &mut border_spans {
            span.style = span.style.bg(bg);
        }
    }
    if is_selected {
        for span in &mut border_spans {
            span.style = span.style.bg(Color::DarkGray);
        }
    }
    lines.push(Line::from(border_spans));

    // Room content: activity description + tools
    let icon = theme::status_icon(&agent.state, anim_frame);

    // Activity line
    let activity = if let Some(ref tool_name) = agent.tool_name {
        if let Some(ref detail) = agent.tool_detail {
            format!("{} {}: {}", theme::ICON_TOOL, tool_name, detail)
        } else {
            format!("{} {}", theme::ICON_TOOL, tool_name)
        }
    } else if let Some(ref emoji) = agent.event_emoji {
        format!("{}", emoji)
    } else {
        "idle".to_string()
    };

    let mut activity_spans = vec![
        Span::styled("│ ".to_string(), border_style),
        Span::styled(format!(" {} ", icon), theme::status_style(&agent.state, true)),
        Span::styled(format!(" {}", activity), dim_style),
    ];
    // Pad to content_width
    let used: usize = activity_spans.iter().map(|s| s.content.width()).sum();
    let pad = content_width.saturating_sub(used + 1); // +1 for │
    activity_spans.push(Span::styled(
        " ".repeat(pad),
        Style::default().fg(Color::Reset),
    ));
    activity_spans.push(Span::styled("│".to_string(), border_style));

    if let Some(bg) = bg_color {
        for span in &mut activity_spans {
            span.style = span.style.bg(bg);
        }
    }
    if is_selected {
        for span in &mut activity_spans {
            span.style = span.style.bg(Color::DarkGray);
        }
    }
    lines.push(Line::from(activity_spans));

    // Tool lines (up to 3)
    let visible_tools: Vec<_> = if agent.tools.len() > 3 {
        agent.tools[agent.tools.len() - 3..].iter().collect()
    } else {
        agent.tools.iter().collect()
    };
    for tool in &visible_tools {
        let tool_text = match &tool.detail {
            Some(detail) => format!("  {}: {}", tool.name, detail),
            None => format!("  {}", tool.name),
        };
        let mut tool_spans = vec![
            Span::styled("│ ".to_string(), border_style),
            Span::styled(format!(" 🔧{}", tool_text), dim_style),
        ];
        let used: usize = tool_spans.iter().map(|s| s.content.width()).sum();
        let pad = content_width.saturating_sub(used + 1);
        tool_spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
        tool_spans.push(Span::styled("│".to_string(), border_style));

        if let Some(bg) = bg_color {
            for span in &mut tool_spans {
                span.style = span.style.bg(bg);
            }
        }
        if is_selected {
            for span in &mut tool_spans {
                span.style = span.style.bg(Color::DarkGray);
            }
        }
        lines.push(Line::from(tool_spans));
    }

    // Duration + summary line
    let duration = format_duration(agent.updated_at);
    let tool_count = if agent.tools.is_empty() {
        String::new()
    } else {
        format!("  📋 {} tools", agent.tools.len())
    };
    let sub_count = if !subagents.is_empty() {
        format!("  🤖 {} sub", subagents.len())
    } else if completed_count > 0 {
        format!("  ✅ {} completed", completed_count)
    } else {
        String::new()
    };
    let info = format!("⏱️ {}{}{}", duration, tool_count, sub_count);

    let mut info_spans = vec![
        Span::styled("│ ".to_string(), border_style),
        Span::styled(format!(" {}", info), dim_style),
    ];
    let used: usize = info_spans.iter().map(|s| s.content.width()).sum();
    let pad = content_width.saturating_sub(used + 1);
    info_spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
    info_spans.push(Span::styled("│".to_string(), border_style));

    if let Some(bg) = bg_color {
        for span in &mut info_spans {
            span.style = span.style.bg(bg);
        }
    }
    if is_selected {
        for span in &mut info_spans {
            span.style = span.style.bg(Color::DarkGray);
        }
    }
    lines.push(Line::from(info_spans));

    // Subagent rooms (compact, inline)
    for sub in subagents {
        let sub_status = match sub.state {
            SubagentStatus::Running => "running",
            SubagentStatus::Waiting => "waiting",
            SubagentStatus::Ended => "ended",
        };
        let sub_emoji = match sub.state {
            SubagentStatus::Running => "🔧",
            SubagentStatus::Waiting => "💤",
            SubagentStatus::Ended => "✅",
        };
        let sub_desc = sub.description.as_deref().unwrap_or(&sub.short_id);
        let sub_tool = if let Some(tool) = sub.tools.first() {
            format!(": {} {}", tool.name, tool.detail.as_deref().unwrap_or(""))
        } else {
            String::new()
        };
        let sub_line = format!("  ├─ 🤖 {} {} {}{}", sub.short_id, sub_emoji, sub_desc, sub_tool);
        let mut sub_spans = vec![
            Span::styled("│ ".to_string(), border_style),
            Span::styled(sub_line, dim_style),
        ];
        let used: usize = sub_spans.iter().map(|s| s.content.width()).sum();
        let pad = content_width.saturating_sub(used + 1);
        sub_spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
        sub_spans.push(Span::styled("│".to_string(), border_style));

        if is_selected {
            for span in &mut sub_spans {
                span.style = span.style.bg(Color::DarkGray);
            }
        }
        lines.push(Line::from(sub_spans));
    }

    // Completed subagents count
    if completed_count > 0 && subagents.is_empty() {
        let comp_line = format!("  └─ ✅ {} completed", completed_count);
        let mut comp_spans = vec![
            Span::styled("│ ".to_string(), border_style),
            Span::styled(comp_line, Style::default().fg(Color::Rgb(0x60, 0x60, 0x60))),
        ];
        let used: usize = comp_spans.iter().map(|s| s.content.width()).sum();
        let pad = content_width.saturating_sub(used + 1);
        comp_spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
        comp_spans.push(Span::styled("│".to_string(), border_style));

        if is_selected {
            for span in &mut comp_spans {
                span.style = span.style.bg(Color::DarkGray);
            }
        }
        lines.push(Line::from(comp_spans));
    }

    // Room bottom border
    let bottom_fill = "─".repeat(content_width.saturating_sub(2));
    let mut bottom_spans = vec![
        Span::styled("└".to_string(), border_style),
        Span::styled(bottom_fill, border_style),
        Span::styled("┘".to_string(), border_style),
    ];
    if let Some(bg) = bg_color {
        for span in &mut bottom_spans {
            span.style = span.style.bg(bg);
        }
    }
    if is_selected {
        for span in &mut bottom_spans {
            span.style = span.style.bg(Color::DarkGray);
        }
    }
    lines.push(Line::from(bottom_spans));

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::AgentStatus;
    use std::collections::HashMap;

    fn make_agent_state(pane_id: &str, status: AgentStatus) -> AgentState {
        AgentState {
            tmux_pane: pane_id.to_string(),
            session_id: None,
            agent_id: None,
            state: status,
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            hook_event_name: Some("PreToolUse".to_string()),
            event_emoji: Some("🔧".to_string()),
            tool_name: Some("Read".to_string()),
            tool_detail: Some("src/main.rs".to_string()),
            tools: vec![crate::agent::state::ToolInfo {
                name: "Read".to_string(),
                detail: Some("src/main.rs".to_string()),
            }],
        }
    }

    #[test]
    fn test_collect_agent_entries_empty() {
        let entries = collect_agent_entries(&[], &HashMap::new(), &HashMap::new());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_collect_agent_entries_with_agent() {
        let mut agent_states = HashMap::new();
        agent_states.insert("%0".to_string(), make_agent_state("%0", AgentStatus::Running));

        let entries = collect_agent_entries(
            &[crate::tmux::types::TmuxSession {
                session_name: "main".to_string(),
                session_attached: true,
                repo_name: None,
                toplevel: None,
                worktree_name: None,
                windows: vec![crate::tmux::types::TmuxWindow {
                    window_index: 0,
                    window_name: "claude".to_string(),
                    window_active: true,
                    panes: vec![crate::tmux::types::TmuxPane {
                        pane_id: "%0".to_string(),
                        pane_index: 0,
                        pane_current_command: "claude".to_string(),
                        pane_current_path: "/home".to_string(),
                        pane_title: String::new(),
                        pane_active: true,
                        agent_state: Some(make_agent_state("%0", AgentStatus::Running)),
                        git_info: None,
                    }],
                }],
            }],
            &agent_states,
            &HashMap::new(),
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_state.state, AgentStatus::Running);
    }

    #[test]
    fn test_format_duration() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(format_duration(now), "0s");
        assert_eq!(format_duration(now - 30), "30s");
        assert_eq!(format_duration(now - 90), "1m 30s");
    }

    #[test]
    fn test_build_office_lines_empty() {
        let lines = build_office_lines(&[], 80, 0, 0);
        assert!(!lines.is_empty()); // Should show "No active agents"
    }

    #[test]
    fn test_build_office_lines_with_entry() {
        let entries = vec![AgentEntry {
            pane_id: "%0".to_string(),
            tmux_target: "main:0.0".to_string(),
            agent_state: make_agent_state("%0", AgentStatus::Running),
            subagents: Vec::new(),
            completed_count: 0,
        }];
        let lines = build_office_lines(&entries, 80, 0, 0);
        assert!(lines.len() > 4); // Border + content + tools + duration + border
    }
}
```

- [ ] **Step 2: Update `src/ui/mod.rs` to export office module**

```rust
pub mod office;
pub mod status_bar;
pub mod theme;
pub mod tree;
```

- [ ] **Step 3: Run tests**

Run: `cargo test ui::office`
Expected: All office tests pass

- [ ] **Step 4: Commit**

```bash
git add src/ui/office.rs src/ui/mod.rs
git commit -m "feat: add office view rendering module"
```

---

### Task 4: Wire office view into App rendering

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Update rendering in `run_app` to delegate based on `view_mode`**

In `src/app.rs`, find the rendering section (around the `tree::render(...)` call) and replace the tree rendering block with a view-mode dispatch. Find this code:

```rust
            // Render tree with inline agent status on single-pane windows
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

Replace with:

```rust
            // Render main content area based on view mode
            let subagent_data = app.build_subagent_data();
            match app.view_mode {
                ViewMode::Tree => {
                    tree::render(
                        f,
                        chunks[1],
                        &app.tree_items,
                        app.selected,
                        app.scroll_offset,
                        app.anim_frame,
                        &subagent_data,
                    );
                }
                ViewMode::Office => {
                    office::render(
                        f,
                        chunks[1],
                        &app.sessions,
                        &app.agent_states,
                        &subagent_data,
                        app.selected,
                        app.scroll_offset,
                        app.anim_frame,
                    );
                }
            }
```

Also add the import at the top of `app.rs`:

```rust
use crate::ui::{office, status_bar, theme, tree};
```

- [ ] **Step 2: Update j/k navigation in office view to clamp to agent entries count**

In `src/app.rs`, the `move_up` and `move_down` methods currently navigate `tree_items`. For office view, we need a different max index. Add a helper method:

```rust
    /// Get the number of selectable items in the current view.
    fn selectable_count(&self) -> usize {
        match self.view_mode {
            ViewMode::Tree => self
                .tree_items
                .iter()
                .filter(|item| item.is_selectable())
                .count()
                .max(1),
            ViewMode::Office => self
                .sessions
                .iter()
                .flat_map(|s| s.windows.iter())
                .flat_map(|w| w.panes.iter())
                .filter(|p| p.agent_state.is_some())
                .count()
                .max(1),
        }
    }
```

Then update `move_up` and `move_down` to use this for office view bounds. Modify `move_up`:

```rust
    fn move_up(&mut self) {
        self.user_navigated = true;
        self.pending_center = true;
        if self.selected > 0 {
            self.selected -= 1;
        }
    }
```

Modify `move_down`:

```rust
    fn move_down(&mut self) {
        self.user_navigated = true;
        self.pending_center = true;
        let max = self.selectable_count().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }
```

- [ ] **Step 3: Handle Enter in office view to switch tmux to selected agent's pane**

Update `handle_select` to work with office view. Add at the beginning of `handle_select`:

```rust
    async fn handle_select(&mut self) -> Result<()> {
        if self.tree_items.is_empty() && self.view_mode == ViewMode::Tree {
            return Ok(());
        }

        if self.view_mode == ViewMode::Office {
            // Find the selected agent's tmux target
            let agent_panes: Vec<_> = self
                .sessions
                .iter()
                .flat_map(|s| {
                    let session_name = s.session_name.clone();
                    s.windows.iter().map(move |w| {
                        let session_name = session_name.clone();
                        w.panes.iter().filter_map(move |p| {
                            p.agent_state.as_ref().map(|_| {
                                format!("{}:{}.{}", session_name, w.window_index, p.pane_index)
                            })
                        })
                    })
                })
                .flatten()
                .collect();

            if let Some(target) = agent_panes.get(self.selected) {
                self.user_navigated = false;
                if let Err(e) = tmux_client::switch_to(target).await {
                    eprintln!("Warning: failed to switch tmux: {}", e);
                }
                self.refresh().await?;
            }
            return Ok(());
        }

        // ... rest of existing handle_select for Tree view ...
```

- [ ] **Step 4: Update title bar to show current view mode**

In the title bar rendering, update the title text to include view mode indicator. Find the non-running title bar rendering (the `else` branch) and update it similarly for both branches. Add a view mode indicator after "chikuwa":

For both the running and non-running title branches, after the `chikuwa` text, add the view indicator. In the `else` (non-running) branch, find:

```rust
                    Span::styled("  chikuwa  ", white_style),
```

Replace with:

```rust
                    Span::styled(
                        match app.view_mode {
                            ViewMode::Tree => "  chikuwa  ",
                            ViewMode::Office => "  chikuwa:office  ",
                        },
                        white_style,
                    ),
```

Do the same for the running branch — find `spans.extend(chikuwa_spans);` and after `"  "` span and before the bolt icon, update the spacing span to include the view label. Find:

```rust
                spans.push(Span::styled("  ", white_style));
                spans.push(Span::styled(theme::ICON_BOLT, bolt_style));
```

Replace with:

```rust
                spans.push(Span::styled(
                    match app.view_mode {
                        ViewMode::Tree => "  ",
                        ViewMode::Office => ":office  ",
                    },
                    white_style,
                ));
                spans.push(Span::styled(theme::ICON_BOLT, bolt_style));
```

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: wire office view into App rendering and navigation"
```

---

### Task 5: Format, lint, and final test

**Files:**
- All modified files

- [ ] **Step 1: Run `cargo fmt`**

Run: `cargo fmt`

- [ ] **Step 2: Run `cargo clippy -- -D warnings`**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run `cargo test`**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit if any formatting changes**

```bash
git add -A
git commit -m "style: format code"
```
