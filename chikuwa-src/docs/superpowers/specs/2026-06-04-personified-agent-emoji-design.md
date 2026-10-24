# Personified Agent Emoji Display

## Goal

Replace text-heavy office view with a visual emoji language that makes agent state instantly recognizable. Agent "personality" is expressed through face emoji (animated per status); tool usage through per-tool animated emoji on separate lines with truncated detail text. Only permission/failure states show additional text notices.

## Architecture

Two-layer animated emoji system in the existing `render_agent_room()`:

1. **Face emoji** — maps from `AgentStatus` + `anim_frame`, 4-frame cycle, replaces the current spinner icon
2. **Tool emoji** — maps from `tool_name` + `anim_frame`, 2-frame cycle per tool, each tool on its own line with truncated detail

New functions in `theme.rs`:
- `agent_face_emoji(status, has_failure, anim_frame) -> &str` — face emoji with frame animation
- `tool_emoji(tool_name, anim_frame) -> &str` — tool emoji with per-tool frame animation
- `event_emoji(hook_event_name) -> &str` — event emoji for title bar right side

## Face Emoji (AgentStatus → Animation Frames, 4-frame cycle)

| AgentStatus | Frame 1 | Frame 2 | Frame 3 | Frame 4 |
|---|---|---|---|---|
| Started | 🟢 | 🆕 | 🟢 | 🆕 |
| Running | 🤔 | 🧐 | 🤔 | 🧐 |
| Permission | 🥺✋ | 😫🙋 | 🥺✋ | 😫🙋 |
| Waiting | 😴 | 💤 | 😴 | 💤 |
| Ended (success) | ✅ | (static) | | |
| Ended (failure) | ❌ | 💥 | ❌ | 💥 |

**Ended success/failure distinction:** `AgentStatus::Ended` doesn't distinguish success vs failure. The face emoji function checks `agent.failure_detail()` — if `Some`, use ❌/💥 frames; if `None`, use ✅ static.

## Tool Emoji (per-tool 2-frame animation)

Each tool has its own 2-frame animation, driven by `anim_frame % 2`. This means different tools on different lines animate independently, creating natural visual rhythm.

| Tool | Frame 1 | Frame 2 | Animation meaning |
|---|---|---|---|
| Bash | 💻 | ⌨️ | Terminal ↔ Keyboard |
| Read | 📖 | 👀 | Book ↔ Eyes |
| Write | 📝 | ✍️ | Notepad ↔ Writing |
| Edit | ✏️ | 🖊️ | Pencil ↔ Pen |
| NotebookEdit | 📓 | ✍️ | Notebook ↔ Writing |
| Grep | 🔍 | 🕵️ | Magnifier ↔ Detective |
| Glob | 📂 | 🗂️ | Folder ↔ Organizer |
| WebFetch | 🌐 | ⬇️ | Globe ↔ Download |
| WebSearch | 🕵️ | 🌐 | Detective ↔ Globe |
| Task / Agent | 👶 | 🥚 | Baby ↔ Hatching |
| AskUserQuestion | ❓ | 🤷 | Question ↔ Shrugging |
| ExitPlanMode | 📋 | ✅ | Plan ↔ Confirm |
| MCP tools | ⚙️ | 🔧 | Gear ↔ Wrench |
| file_edited (OpenCode) | ✏️ | 🖊️ | Pencil ↔ Pen |
| Unknown | 🔧 | ⚙️ | Wrench ↔ Gear |

## Hook Event Emoji (title bar right side, static)

| Event | Emoji |
|---|---|
| PreToolUse | 🪝 |
| PostToolUse (success) | 🟩 |
| PostToolUseFailure | 🟥 |
| UserPromptSubmit | ✍️ |
| SubagentStart | 👶 |
| SubagentStop | 🔀 |
| Stop (success) | ✅ |
| Stop (failure) | ❌ |
| PermissionRequest | 🔐 |
| PreCompact | 🗜️ |

## Room Layout

### Running + 3 tools (animated)

Frame 1:
```
┌─ 🤔 ──── 🪝 ──────────────────────┐
│                                      │
│ 💻 cargo test                        │
│ 📖 src/main.rs:42                    │
│ 🔍 TODO.*fix                         │
│ ⏱️ 2m                               │
│                                      │
└──────────────────────────────────────┘
```

Frame 2:
```
┌─ 🧐 ──── 🪝 ──────────────────────┐
│                                      │
│ ⌨️ cargo test                        │
│ 👀 src/main.rs:42                    │
│ 🕵️ TODO.*fix                         │
│ ⏱️ 2m                               │
│                                      │
└──────────────────────────────────────┘
```

### Permission + Edit tool (animated)

Frame 1:
```
┌─ 🥺 ──── ✋ ──────────────────────┐
│                                      │
│ ✏️ src/api/auth.rs                   │
│ needs input! ⚠️                     │
│ ⏱️ 5m                               │
│                                      │
└──────────────────────────────────────┘
```

Frame 2:
```
┌─ 😫 ──── 🙋 ──────────────────────┐
│                                      │
│ 🖊️ src/api/auth.rs                   │
│ needs input! ⚠️                     │
│ ⏱️ 5m                               │
│                                      │
└──────────────────────────────────────┘
```

### Waiting (no active tools)

```
┌─ 😴 ─────────────────────────────┐
│                                     │
│ ⏱️ 5m                              │
│                                     │
└─────────────────────────────────────┘
```

### Detail truncation rules

- Each tool line: `emoji + detail_text`, truncated with `…` if exceeding room width
- Maximum 3 tool lines shown; excess shown as `+N more`
- Detail text is the existing `tool_detail` field (file path, command, pattern, etc.)

### Room structure changes from current

- Title bar: face emoji (left) + event emoji (right), remove agent name text and status label text
- Tool lines: animated tool emoji + truncated detail (one line per tool), replacing current text-heavy `🔧 Bash: cargo test`
- Duration line: `⏱️ Xm` only, remove `📋 N tools` text
- Permission/failure: show text notice after tool lines
- Empty line padding above/below content preserved

## Implementation Scope

### Files Modified

1. **`src/ui/theme.rs`** — Add `agent_face_emoji()`, `tool_emoji()`, `event_emoji()` functions
2. **`src/ui/office.rs`** — Rewrite `render_agent_room()` to use emoji language, per-tool animated lines with truncated detail

### No Changes Needed

- `src/agent/state.rs` — No data model changes
- `src/app.rs` — No logic changes
- `plugins/chikuwa.ts` — Plugin already sends correct data
