# PostToolUse/PostToolUseFailure Display Behavior

## Goal

Change how PostToolUse and PostToolUseFailure events appear in the TUI:
- PostToolUse: silently remove the tool from the active list without showing a ✅ status
- PostToolUseFailure: show a red failure line with truncated output extracted from the hook input

## Architecture

### DisplayMode enum (replaces `suppress: bool` in ParseResult)

```rust
pub enum DisplayMode {
    Show,    // Normal display — update state, emoji, tools, everything
    Silent,  // Update tools list only — don't change visual state (emoji, status, tool_name)
    Suppress, // Ignore completely — don't send to TUI at all
}
```

### PostToolUse behavior

- Parser returns `DisplayMode::Silent`
- IPC message is still sent so the TUI can remove the tool from the active list
- TUI updates `tools` but preserves `state`, `event_emoji`, `tool_name`, `tool_detail`, `failure_detail`

### PostToolUseFailure behavior

- Parser returns `DisplayMode::Show`
- Extract failure detail from hook input `message` field (fallback to `tool_name`)
- Store in `AgentState.failure_detail: Option<String>`
- UI renders as a red line with ❌ icon
- `failure_detail` is cleared on the next non-PostToolUseFailure event

### New fields and constants

- `AgentState.failure_detail: Option<String>` — truncated failure message
- `theme::COLOR_FAILURE: Color = Color::Rgb(0xff, 0x44, 0x44)` — red for failure text

### Failure detail extraction

From `ClaudeHookInput.message` field (already deserialized but unused). If the message contains useful text (non-empty, not just "permission_prompt"), use it truncated to ~80 chars. Otherwise fall back to `"tool_name failed"`.

### UI rendering

**Tree view**: When `failure_detail` is set, render an extra line below the agent status:
```
  │  ❌ Edit: src/main.rs — Permission denied
```
The line uses `COLOR_FAILURE` for the text. The line disappears when `failure_detail` is cleared.

**Office view**: Same failure line rendered inside the agent room, in red.

### Failure detail lifecycle

1. PostToolUseFailure sets `failure_detail`
2. Any subsequent non-PostToolUseFailure event (PreToolUse, Stop, UserPromptSubmit, etc.) clears it
3. This ensures the failure message is visible temporarily, then disappears as the agent continues

## Files to modify

| File | Change |
|------|--------|
| `src/agent/parser.rs` | Replace `suppress: bool` with `display: DisplayMode`; PostToolUse → Silent; extract failure_detail |
| `src/agent/state.rs` | Add `failure_detail: Option<String>` to AgentState |
| `src/hook.rs` | Update to use `display` field instead of `suppress` |
| `src/opencode.rs` | Update to use `display` field instead of `suppress` |
| `src/app.rs` | Handle DisplayMode::Silent (update tools only, preserve visual state); clear failure_detail on non-failure events |
| `src/ui/theme.rs` | Add COLOR_FAILURE constant |
| `src/ui/tree.rs` | Render failure_detail line in red |
| `src/ui/office.rs` | Render failure_detail line in red |
| `src/persist.rs` | Add failure_detail: None to test AgentState constructions |
