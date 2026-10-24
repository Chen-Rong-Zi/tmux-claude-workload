# Unified Subagent Display + Time-Escalated Emoji Animation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Embed subagent lines inside main agent room, replace per-tool emoji with ◐ spinner, add time-escalated face emoji for Permission/Waiting states, add contextual duration labels.

**Architecture:** Update `theme.rs` with new spinner/time-escalation functions, then rewrite `render_agent_room()` in `office.rs` to embed subagent lines and use the new emoji system. Delete `render_subagent_cards()`.

**Tech Stack:** Rust, ratatui, unicode-width

---

### Task 1: Update theme.rs — Replace tool_emoji with spinner, add time-escalation helpers

**Files:**
- Modify: `src/ui/theme.rs`

- [ ] **Step 1: Add TOOL_SPINNER_FRAMES constant and tool_spinner function, remove tool_emoji**

Replace the `tool_emoji` function (lines 95–124) with:

```rust
pub const TOOL_SPINNER_FRAMES: &[&str] = &["◐", "◓", "◑", "◒"];

pub fn tool_spinner(anim_frame: usize) -> &'static str {
    TOOL_SPINNER_FRAMES[anim_frame % TOOL_SPINNER_FRAMES.len()]
}
```

- [ ] **Step 2: Update agent_face_emoji signature and body**

Replace the `agent_face_emoji` function (lines 68–92) with:

```rust
/// Face emoji for agent status, animated only when working or needing attention.
/// Running and Permission animate; all other states are static.
/// Time-escalated: Permission and Waiting frames intensify with elapsed time.
pub fn agent_face_emoji(
    status: &AgentStatus,
    has_failure: bool,
    anim_frame: usize,
    elapsed_secs: u64,
) -> &'static str {
    match status {
        AgentStatus::Started => "🟢",
        AgentStatus::Running => {
            let frames = ["⚙️", "🔧"];
            frames[anim_frame % frames.len()]
        }
        AgentStatus::Permission => {
            let frames = if elapsed_secs < 30 {
                ["🥺✋", "🙁🤚"]
            } else if elapsed_secs < 60 {
                ["😯🙋", "😫🙋"]
            } else {
                ["😫🙋", "😫🙋‍♂️"]
            };
            frames[anim_frame % frames.len()]
        }
        AgentStatus::Waiting => {
            let frames = if elapsed_secs < 30 {
                ["😴", "😪"]
            } else if elapsed_secs < 90 {
                ["😪", "🥱"]
            } else {
                ["🥱", "😵‍💫"]
            };
            frames[anim_frame % frames.len()]
        }
        AgentStatus::Ended => {
            if has_failure { "❌" } else { "✅" }
        }
    }
}
```

- [ ] **Step 3: Add permission_warning_text and idle_zzz_count functions**

Add after `agent_face_emoji`:

```rust
/// Permission warning text, escalating with wait time.
pub fn permission_warning_text(elapsed_secs: u64) -> &'static str {
    if elapsed_secs < 30 {
        "🟡 NEED USER INPUT ⚠️"
    } else if elapsed_secs < 60 {
        "🟠 AWAITING YOUR RESPONSE ⚠️"
    } else {
        "🔴 PLEASE INPUT ASAP ⚠️"
    }
}

/// Number of 💤 emojis for idle state, based on elapsed time.
pub fn idle_zzz_count(elapsed_secs: u64) -> usize {
    if elapsed_secs < 30 {
        1
    } else if elapsed_secs < 90 {
        2
    } else {
        3
    }
}

/// Duration label prefix based on agent status.
pub fn duration_label(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "Running",
        AgentStatus::Permission => "Waited",
        AgentStatus::Waiting => "Idle",
        AgentStatus::Started => "",
        AgentStatus::Ended => "Done",
    }
}
```

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test && cargo clippy -- -D warnings`
Expected: May have compilation errors in office.rs due to `tool_emoji` removal and `agent_face_emoji` signature change — that's expected, will fix in Task 2.

- [ ] **Step 5: Commit**

```bash
git add src/ui/theme.rs
git commit -m "feat: add spinner, time-escalated face emoji, and duration labels"
```

---

### Task 2: Update office.rs — Fix compilation errors from theme.rs changes

**Files:**
- Modify: `src/ui/office.rs`

- [ ] **Step 1: Update agent_face_emoji call in render_agent_room**

In `render_agent_room()`, the call on line 464 needs the new `elapsed_secs` parameter. Replace:

```rust
let face = theme::agent_face_emoji(&agent.status(), has_failure, anim_frame);
```

with:

```rust
let elapsed_secs = elapsed_secs(agent.updated_at);
let face = theme::agent_face_emoji(&agent.status(), has_failure, anim_frame, elapsed_secs);
```

Add this helper function (can go near `format_duration`):

```rust
/// Compute elapsed seconds since a Unix timestamp.
fn elapsed_secs(updated_at: u64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.saturating_sub(updated_at)
}
```

- [ ] **Step 2: Replace tool_emoji calls with tool_spinner**

In `render_agent_room()`, replace the two `tool_emoji` calls:

Line 471 — title bar right emoji:
```rust
theme::tool_emoji(tool_name, anim_frame)
```
becomes:
```rust
theme::tool_spinner(anim_frame)
```

Line 519 — tool line emoji:
```rust
let emoji = theme::tool_emoji(&tool.name, anim_frame + i);
```
becomes:
```rust
let emoji = theme::tool_spinner(anim_frame + i);
```

- [ ] **Step 3: Run tests to verify compilation**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/ui/office.rs
git commit -m "refactor: update office.rs to use tool_spinner and new face emoji signature"
```

---

### Task 3: Rewrite render_agent_room — embed subagent lines, duration labels, permission warning, 💤

**Files:**
- Modify: `src/ui/office.rs`

- [ ] **Step 1: Rewrite render_agent_room completely**

Replace the entire `render_agent_room` function with this implementation:

```rust
fn render_agent_room(
    agent: &AgentState,
    subagents: &[SubagentInfo],
    completed_count: u32,
    content_width: usize,
    is_selected: bool,
    is_permission: bool,
    anim_frame: usize,
) -> Vec<Line<'static>> {
    let dim_style = Style::default().fg(theme::COLOR_DIM);
    let name_style = if is_permission {
        Style::default()
            .fg(theme::COLOR_WHITE)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::COLOR_LIGHT_PURPLE)
    };

    let bg_color = if is_permission {
        Some(theme::COLOR_PERMISSION_BG)
    } else {
        None
    };

    let border_style = if is_permission {
        Style::default().fg(theme::COLOR_WHITE)
    } else {
        Style::default().fg(theme::COLOR_PURPLE)
    };

    let has_failure = agent.failure_detail().is_some();
    let elapsed = elapsed_secs(agent.updated_at);
    let face = theme::agent_face_emoji(&agent.status(), has_failure, anim_frame, elapsed);

    // Right indicator for title bar
    let right_indicator = match agent.status() {
        AgentStatus::Running => theme::tool_spinner(anim_frame).to_string(),
        AgentStatus::Permission => String::new(),
        AgentStatus::Waiting => "💤".repeat(theme::idle_zzz_count(elapsed)),
        _ => agent
            .event_emoji()
            .unwrap_or_else(|| theme::event_emoji(agent.event_label()))
            .to_string(),
    };

    let mut lines = Vec::new();

    // Title bar: ┌─ face ──── right_indicator ──┐
    let face_width = face.width();
    let right_width = right_indicator.width();
    let inner_width = content_width.saturating_sub(2); // ┌ and ┐
    let fill_len = if right_indicator.is_empty() {
        inner_width.saturating_sub(face_width + 2) // ┌─ face ──┐
    } else {
        inner_width.saturating_sub(face_width + right_width + 4) // ┌─ face ──── right ──┐
    };
    let fill = "─".repeat(fill_len);

    let mut border_spans = vec![
        Span::styled("┌─ ".to_string(), border_style),
        Span::styled(face.to_string(), name_style),
    ];
    if right_indicator.is_empty() {
        border_spans.push(Span::styled(format!(" {}┐", fill), border_style));
    } else {
        border_spans.push(Span::styled(format!(" {} ", fill), border_style));
        border_spans.push(Span::styled(right_indicator, dim_style));
        border_spans.push(Span::styled(" ─┐".to_string(), border_style));
    }
    apply_bg(&mut border_spans, bg_color, is_selected);
    lines.push(Line::from(border_spans));

    // Empty line after title
    lines.push(make_empty_line(content_width, border_style, bg_color, is_selected));

    // Tool lines (all with ◐ spinner)
    let active_tools = agent.active_tools();
    for (i, tool) in active_tools.iter().enumerate() {
        let spinner = theme::tool_spinner(anim_frame + i);
        let detail = tool.detail.as_deref().unwrap_or("");
        let tool_text = format!("{} {}", spinner, detail);
        let tool_text = truncate_to_width(&tool_text, content_width.saturating_sub(4));
        let mut tool_spans = vec![
            Span::styled("│ ".to_string(), border_style),
            Span::styled(format!(" {}", tool_text), dim_style),
        ];
        let used: usize = tool_spans.iter().map(|s| s.content.width()).sum();
        let pad = content_width.saturating_sub(used + 1);
        tool_spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
        tool_spans.push(Span::styled("│".to_string(), border_style));
        apply_bg(&mut tool_spans, bg_color, is_selected);
        lines.push(Line::from(tool_spans));
    }

    // Empty line before subagents (if any subagents or completed count)
    if !subagents.is_empty() || completed_count > 0 {
        lines.push(make_empty_line(content_width, border_style, bg_color, is_selected));
    }

    // Subagent lines
    for (si, sub) in subagents.iter().enumerate() {
        let sub_elapsed = elapsed_secs(sub.updated_at);
        let sub_status = match sub.state {
            SubagentStatus::Running => AgentStatus::Running,
            SubagentStatus::Waiting => AgentStatus::Permission,
            SubagentStatus::Ended => AgentStatus::Ended,
        };
        let sub_face = theme::agent_face_emoji(
            &sub_status,
            false,
            anim_frame + si,
            sub_elapsed,
        );
        let duration = format_duration(sub.updated_at);

        if sub.tools.is_empty() {
            // No tools: │ 👶 face    duration │
            let text = format!("👶 {}  {}", sub_face, duration);
            let text = truncate_to_width(&text, content_width.saturating_sub(4));
            let mut spans = vec![
                Span::styled("│ ".to_string(), border_style),
                Span::styled(format!(" {}", text), dim_style),
            ];
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            let pad = content_width.saturating_sub(used + 1);
            spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
            spans.push(Span::styled("│".to_string(), border_style));
            apply_bg(&mut spans, bg_color, is_selected);
            lines.push(Line::from(spans));
        } else {
            // First tool: │ 👶 face spinner detail  duration │
            let first_tool = &sub.tools[0];
            let spinner = theme::tool_spinner(anim_frame + si);
            let detail = first_tool.detail.as_deref().unwrap_or("");
            let text = format!("👶 {} {} {}", sub_face, spinner, detail);
            let dur_text = format!("  {}", duration);
            let max_detail_width = content_width
                .saturating_sub(4)
                .saturating_sub(dur_text.width());
            let text = truncate_to_width(&text, max_detail_width);
            let mut spans = vec![
                Span::styled("│ ".to_string(), border_style),
                Span::styled(format!(" {}", text), dim_style),
            ];
            // Right-align duration
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            let pad = content_width.saturating_sub(used + dur_text.width() + 1);
            spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
            spans.push(Span::styled(dur_text, dim_style));
            spans.push(Span::styled("│".to_string(), border_style));
            apply_bg(&mut spans, bg_color, is_selected);
            lines.push(Line::from(spans));

            // Subsequent tools: │    spinner detail │
            for (ti, tool) in sub.tools[1..].iter().enumerate() {
                let spinner = theme::tool_spinner(anim_frame + si + ti + 1);
                let detail = tool.detail.as_deref().unwrap_or("");
                let text = format!("   {} {}", spinner, detail);
                let text = truncate_to_width(&text, content_width.saturating_sub(4));
                let mut spans = vec![
                    Span::styled("│".to_string(), border_style),
                    Span::styled(format!(" {}", text), dim_style),
                ];
                let used: usize = spans.iter().map(|s| s.content.width()).sum();
                let pad = content_width.saturating_sub(used + 1);
                spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
                spans.push(Span::styled("│".to_string(), border_style));
                apply_bg(&mut spans, bg_color, is_selected);
                lines.push(Line::from(spans));
            }
        }
    }

    // Completed subagent count
    if completed_count > 0 {
        let text = format!("✓ {} completed", completed_count);
        let mut spans = vec![
            Span::styled("│ ".to_string(), border_style),
            Span::styled(format!(" {}", text), dim_style),
        ];
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let pad = content_width.saturating_sub(used + 1);
        spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
        spans.push(Span::styled("│".to_string(), border_style));
        apply_bg(&mut spans, bg_color, is_selected);
        lines.push(Line::from(spans));
    }

    // Permission warning text
    if agent.status() == AgentStatus::Permission {
        let warning = theme::permission_warning_text(elapsed);
        let mut spans = vec![
            Span::styled("│ ".to_string(), border_style),
            Span::styled(format!(" {}", warning), name_style),
        ];
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let pad = content_width.saturating_sub(used + 1);
        spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
        spans.push(Span::styled("│".to_string(), border_style));
        apply_bg(&mut spans, bg_color, is_selected);
        lines.push(Line::from(spans));
    }

    // Failure detail line
    if let Some(failure) = agent.failure_detail() {
        let failure_style = Style::default().fg(theme::COLOR_FAILURE);
        let mut spans = vec![
            Span::styled("│ ".to_string(), border_style),
            Span::styled(format!(" 💥 {}", failure), failure_style),
        ];
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let pad = content_width.saturating_sub(used + 1);
        spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
        spans.push(Span::styled("│".to_string(), border_style));
        apply_bg(&mut spans, bg_color, is_selected);
        lines.push(Line::from(spans));
    }

    // Empty line before duration
    lines.push(make_empty_line(content_width, border_style, bg_color, is_selected));

    // Duration line with contextual label
    let label = theme::duration_label(&agent.status());
    let duration = format_duration(agent.updated_at);
    let info = if label.is_empty() {
        format!("⏱️ {}", duration)
    } else {
        format!("⏱️ {}: {}", label, duration)
    };
    let mut info_spans = vec![
        Span::styled("│ ".to_string(), border_style),
        Span::styled(format!(" {}", info), dim_style),
    ];
    let used: usize = info_spans.iter().map(|s| s.content.width()).sum();
    let pad = content_width.saturating_sub(used + 1);
    info_spans.push(Span::styled(" ".repeat(pad), Style::default().fg(Color::Reset)));
    info_spans.push(Span::styled("│".to_string(), border_style));
    apply_bg(&mut info_spans, bg_color, is_selected);
    lines.push(Line::from(info_spans));

    // Room bottom border
    let bottom_fill = "─".repeat(content_width.saturating_sub(2));
    let mut bottom_spans = vec![
        Span::styled("└".to_string(), border_style),
        Span::styled(bottom_fill, border_style),
        Span::styled("┘".to_string(), border_style),
    ];
    apply_bg(&mut bottom_spans, bg_color, is_selected);
    lines.push(Line::from(bottom_spans));

    lines
}
```

- [ ] **Step 2: Update the call site in build_office_lines**

In `build_office_lines()`, update the `render_agent_room` call (around line 201) and remove the separate subagent card rendering block. Replace:

```rust
// Main agent room
let room_lines = render_agent_room(
    &entry.agent_state,
    content_width,
    is_selected,
    is_permission,
    anim_frame,
);
lines.extend(room_lines);

// Subagent cards below the main room
if !entry.subagents.is_empty() {
    lines.push(Line::from(""));
    let card_lines = render_subagent_cards(&entry.subagents, content_width, anim_frame);
    lines.extend(card_lines);
}
```

with:

```rust
// Main agent room (includes subagent lines)
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
```

- [ ] **Step 3: Delete render_subagent_cards function**

Delete the entire `render_subagent_cards` function (lines 264–433).

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass. Some office tests may need updating due to the new render_agent_room signature.

- [ ] **Step 5: Fix any failing office tests**

The test `test_build_office_lines_with_entry` and `test_build_office_lines_permission_highlight` create `AgentEntry` and call `build_office_lines`. These should still work since `build_office_lines` passes the subagents internally. Verify and fix any failures.

- [ ] **Step 6: Commit**

```bash
git add src/ui/office.rs
git commit -m "feat: embed subagent lines in agent room with spinner and time-escalated emoji"
```

---

### Task 4: Add unit tests for new theme functions

**Files:**
- Modify: `src/ui/theme.rs`

- [ ] **Step 1: Add tests for tool_spinner, agent_face_emoji time escalation, permission_warning_text, idle_zzz_count, duration_label**

Append to the existing `#[cfg(test)] mod tests` block in theme.rs (or create one if it doesn't exist):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_spinner_cycles() {
        assert_eq!(tool_spinner(0), "◐");
        assert_eq!(tool_spinner(1), "◓");
        assert_eq!(tool_spinner(2), "◑");
        assert_eq!(tool_spinner(3), "◒");
        assert_eq!(tool_spinner(4), "◐"); // wraps
    }

    #[test]
    fn test_agent_face_emoji_started_static() {
        assert_eq!(
            agent_face_emoji(&AgentStatus::Started, false, 0, 0),
            "🟢"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Started, false, 1, 100),
            "🟢"
        );
    }

    #[test]
    fn test_agent_face_emoji_running_animated() {
        assert_eq!(
            agent_face_emoji(&AgentStatus::Running, false, 0, 0),
            "⚙️"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Running, false, 1, 0),
            "🔧"
        );
    }

    #[test]
    fn test_agent_face_emoji_permission_escalation() {
        // 0-30s: mild
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 0, 0),
            "🥺✋"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 1, 29),
            "🙁🤚"
        );
        // 30-60s: urgent
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 0, 30),
            "😯🙋"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 1, 59),
            "😫🙋"
        );
        // 60s+: critical
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 0, 60),
            "😫🙋"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Permission, false, 1, 120),
            "😫🙋‍♂️"
        );
    }

    #[test]
    fn test_agent_face_emoji_waiting_escalation() {
        // 0-30s: light drowsiness
        assert_eq!(
            agent_face_emoji(&AgentStatus::Waiting, false, 0, 0),
            "😴"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Waiting, false, 1, 29),
            "😪"
        );
        // 30-90s: yawning
        assert_eq!(
            agent_face_emoji(&AgentStatus::Waiting, false, 0, 30),
            "😪"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Waiting, false, 1, 89),
            "🥱"
        );
        // 90s+: deep sleep
        assert_eq!(
            agent_face_emoji(&AgentStatus::Waiting, false, 0, 90),
            "🥱"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Waiting, false, 1, 200),
            "😵‍💫"
        );
    }

    #[test]
    fn test_agent_face_emoji_ended() {
        assert_eq!(
            agent_face_emoji(&AgentStatus::Ended, false, 0, 0),
            "✅"
        );
        assert_eq!(
            agent_face_emoji(&AgentStatus::Ended, true, 0, 0),
            "❌"
        );
    }

    #[test]
    fn test_permission_warning_text() {
        assert_eq!(permission_warning_text(0), "🟡 NEED USER INPUT ⚠️");
        assert_eq!(permission_warning_text(29), "🟡 NEED USER INPUT ⚠️");
        assert_eq!(permission_warning_text(30), "🟠 AWAITING YOUR RESPONSE ⚠️");
        assert_eq!(permission_warning_text(59), "🟠 AWAITING YOUR RESPONSE ⚠️");
        assert_eq!(permission_warning_text(60), "🔴 PLEASE INPUT ASAP ⚠️");
        assert_eq!(permission_warning_text(300), "🔴 PLEASE INPUT ASAP ⚠️");
    }

    #[test]
    fn test_idle_zzz_count() {
        assert_eq!(idle_zzz_count(0), 1);
        assert_eq!(idle_zzz_count(29), 1);
        assert_eq!(idle_zzz_count(30), 2);
        assert_eq!(idle_zzz_count(89), 2);
        assert_eq!(idle_zzz_count(90), 3);
        assert_eq!(idle_zzz_count(300), 3);
    }

    #[test]
    fn test_duration_label() {
        assert_eq!(duration_label(&AgentStatus::Running), "Running");
        assert_eq!(duration_label(&AgentStatus::Permission), "Waited");
        assert_eq!(duration_label(&AgentStatus::Waiting), "Idle");
        assert_eq!(duration_label(&AgentStatus::Started), "");
        assert_eq!(duration_label(&AgentStatus::Ended), "Done");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add src/ui/theme.rs
git commit -m "test: add unit tests for spinner, time-escalated emoji, and duration labels"
```

---

### Task 5: Final verification — fmt, clippy, full test suite

**Files:**
- None (verification only)

- [ ] **Step 1: Run cargo fmt**

Run: `cargo fmt`

- [ ] **Step 2: Run cargo clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit any formatting changes**

```bash
git add -A
git commit -m "style: format code" # only if there are changes
```
