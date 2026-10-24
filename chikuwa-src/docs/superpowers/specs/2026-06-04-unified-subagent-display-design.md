# Unified Subagent Display + Time-Escalated Emoji Animation

## Goal

1. Embed subagent display inside the main agent room as single-line rows
2. Replace per-tool emoji with universal ◐◓◑◒ spinner
3. Time-escalated face emoji for Permission and Waiting states (longer wait = more urgent)
4. Contextual duration labels (Idle/Waited/Running)
5. Unified visual language across all three panel states

## Architecture — Three Panel States

### Running (tools executing)

```
┌─ ⚙️ ──── ◐ ───────────────────────┐
│                                      │
│ ◐ cargo test                         │
│ ◐ src/main.rs:42                     │
│                                      │
│ 👶 🤔 ◐ search code          30s    │
│    ◐ read file.rs                     │
│ 👶 😴                  1m           │
│ ✓ 2 completed                        │
│                                      │
│ ⏱️ Running: 5m                       │
└──────────────────────────────────────┘
```

Face emoji: ⚙️ / 🔧 (2-frame cycle, 500ms)
Title bar right: ◐ spinner for current tool
Duration label: `Running: {time}`

### Permission (needs user input) — time-escalated

**0–30s (mild, gentle wave):**
```
┌─ 🥺✋ ────────────────────────┐
│                                 │
│ 🟡 NEED USER INPUT ⚠️          │
│                                 │
│ ⏱️ Waited: 15s                  │
└─────────────────────────────────┘
```
Face frames: 🥺✋ / 🙁🤚 (2-frame cycle)

**30–60s (urgent, raising hand):**
```
┌─ 😯🙋 ────────────────────────┐
│                                 │
│ 🟠 AWAITING YOUR RESPONSE ⚠️   │
│                                 │
│ ⏱️ Waited: 45s                  │
└─────────────────────────────────┘
```
Face frames: 😯🙋 / 😫🙋 (2-frame cycle)

**60s+ (critical):**
```
┌─ 😫🙋 ────────────────────────┐
│                                 │
│ 🔴 PLEASE INPUT ASAP ⚠️        │
│                                 │
│ ⏱️ Waited: 2m 30s              │
└─────────────────────────────────┘
```
Face frames: 😫🙋 / 😫🙋‍♂️ (2-frame cycle)

Duration label: `Waited: {time}`
Warning text uses emoji indicators (🟡🟠🔴) for color distinction without breaking the 3-color palette.

### Waiting (idle) — time-escalated

**0–30s (light drowsiness, 1 💤):**
```
┌─ 😴 ─────────────────────── 💤┐
│                                 │
│ ⏱️ Idle: 15s                    │
└─────────────────────────────────┘
```
Face frames: 😴 / 😪 (2-frame cycle)

**30–90s (yawning, 2 💤):**
```
┌─ 😪 ────────────────────── 💤💤┐
│                                 │
│ ⏱️ Idle: 55s                    │
└─────────────────────────────────┘
```
Face frames: 😪 / 🥱 (2-frame cycle)

**90s+ (deep sleep, 3 💤):**
```
┌─ 🥱 ─────────────────── 💤💤💤┐
│                                 │
│ ⏱️ Idle: 3m 20s                │
└─────────────────────────────────┘
```
Face frames: 🥱 / 😵‍💫 (2-frame cycle)

Duration label: `Idle: {time}`
Right side 💤 count increases with idle duration.

### Other states (static, no animation)

| State | Face | Duration label |
|-------|------|---------------|
| Started | 🟢 | — |
| Ended (success) | ✅ | `Done: {time}` |
| Ended (failure) | ❌ | `Done: {time}` |

## Face Emoji Summary

| State | 0–30s | 30–60s | 60s+ |
|-------|-------|--------|------|
| Running | ⚙️/🔧 (always) | — | — |
| Permission | 🥺✋/🙁🤚 | 😯🙋/😫🙋 | 😫🙋/😫🙋‍♂️ |
| Waiting | 😴/😪 (💤) | 😪/🥱 (💤💤) | 🥱/😵‍💫 (💤💤💤) |
| Started | 🟢 (static) | — | — |
| Ended | ✅/❌ (static) | — | — |

## Spinner (running tool indicator)

4-frame cycle: `◐ ◓ ◑ ◒`

All running tools display this spinner. Replaces `tool_emoji()`.

## Subagent Lines (inside main room)

**First line (has tools):** `│ 👶 {face_emoji} ◐ {detail}  {duration}`

- `👶` prefix identifies subagent
- `face_emoji` from `agent_face_emoji()` mapping SubagentStatus → AgentStatus:
  - Running → `AgentStatus::Running` (⚙️/🔧 animated)
  - Waiting → `AgentStatus::Permission` (🥺✋/😫🙋 animated, time-escalated)
  - Ended → `AgentStatus::Ended` (✅/❌ static)
- ◐ spinner for running tools (animated)
- Duration right-aligned when space allows

**Subsequent tool lines:** `│    ◐ {detail}`

- 2-space indent, aligning with first tool column

**No tools:** `│ 👶 {face_emoji}  {duration}`

### Completed subagent count

`│ ✓ {count} completed` — only if `completed_count > 0`

### Animation offsets

- Main agent tools: `anim_frame`
- Subagent face + first tool: `anim_frame + subagent_index`
- Subagent subsequent tools: `anim_frame + subagent_index + tool_index`

## Title Bar Format

`┌─ {face_emoji} ──── {right_indicator} ──┐`

- No extra short dashes between face and fill line
- Running: right = ◐ spinner
- Permission: right = empty (face already shows ✋/🙋)
- Waiting: right = 💤 × count (1/2/3 based on idle duration)
- Started/Ended: right = event emoji

## Duration Labels

| State | Label format |
|-------|-------------|
| Running | `⏱️ Running: {time}` |
| Permission | `⏱️ Waited: {time}` |
| Waiting | `⏱️ Idle: {time}` |
| Started | (no duration) |
| Ended | `⏱️ Done: {time}` |

## Permission Warning Text

Line after tool lines (or empty line if no tools), only when status is Permission:

| Wait time | Text |
|-----------|------|
| 0–30s | `🟡 NEED USER INPUT ⚠️` |
| 30–60s | `🟠 AWAITING YOUR RESPONSE ⚠️` |
| 60s+ | `🔴 PLEASE INPUT ASAP ⚠️` |

## Time Calculation

Elapsed time computed from `agent.updated_at` (Unix timestamp of last state transition). The `agent_face_emoji()` function signature changes to accept `elapsed_secs: u64` for time-escalated frame selection.

## Files Modified

1. **`src/ui/theme.rs`**
   - Remove `tool_emoji()` function
   - Add `TOOL_SPINNER_FRAMES: &[&str] = &["◐", "◓", "◑", "◒"]`
   - Add `tool_spinner(anim_frame: usize) -> &'static str`
   - Update `agent_face_emoji()` signature: add `elapsed_secs: u64`
   - Update `agent_face_emoji()` body: time-escalated frames for Permission and Waiting, ⚙️/🔧 for Running
   - Add `permission_warning_text(elapsed_secs: u64) -> &'static str`
   - Add `idle_zzz_count(elapsed_secs: u64) -> usize` (returns 1/2/3)

2. **`src/ui/office.rs`**
   - Rewrite `render_agent_room()` to embed subagent lines
   - Replace `tool_emoji()` calls with `tool_spinner()`
   - Add duration labels (Running/Waited/Idle/Done)
   - Add permission warning text line
   - Add right-side 💤 for waiting state
   - Delete `render_subagent_cards()` function
   - Clean up title bar format (remove extra dashes)

## No Changes Needed

- `src/agent/subagent.rs` — No data model changes
- `src/agent/state.rs` — No changes
- `plugins/chikuwa.ts` — No changes
