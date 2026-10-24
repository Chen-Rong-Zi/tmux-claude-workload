# OpenCode IPC Contract Design

**Date:** 2026-06-13
**Status:** Approved
**Goal:** Prevent cross-file inconsistencies between OpenCode TypeScript plugin and Rust
TUI through code generation, schema validation, and integration tests.

## Problem

The OpenCode integration spans 7+ files across TypeScript and Rust:

| Layer | Files | Inconsistency risk |
|---|---|---|
| IPC serialization | `plugins/chikuwa.ts`, `src/agent/state.rs`, `src/agent/opencode_state.rs` | Field names, types, serde tags |
| Detection | `src/agent/detect.rs` | Window name / pane title matching rules |
| Merge logic | `src/agent/opencode_state.rs` | Event type mapping, tool add/remove |
| UI rendering | `src/ui/theme.rs`, `src/ui/tree.rs` | Icon dispatch, status display |

Historical bugs caused by mismatches:
- `serde(rename = "open_code")` vs plugin sending `"opencode"` — serialization tag mismatch
- `is_opencode_command()` only checking `node` command, but tools change pane command to `bash`
- Stale `"running"` state persisting after OpenCode exits

## Architecture

Three-layer defense, all work done before runtime:

```
Dev time:
  Rust structs (canonical)
    └─ cargo run -- generate-contract ──→ opencode-types.ts
    └─ schemars                         ──→ opencode-protocol.json

Test time (cargo test):
  ├─ test_contract_is_up_to_date  ── schema + types vs committed
  ├─ integration tests             ── full IPC path deserialize + merge + render

Runtime (zero validation):
  TS plugin:    JSON.stringify(state) + "\n" → send as-is
  Rust:         serde_json::from_str (standard, no deny_unknown_fields)
```

## Components

### A — Code Generator (`src/bin/gen_contract.rs`)

A single Rust binary that reads canonical struct definitions and outputs two files.

**Input structs (canonical):**
- `AgentState` — `tmux_pane`, `updated_at`, `data: AgentData::OpenCode(OpenCodeState)`
- `OpenCodeState` — `session_id`, `status`, `event_type`, `event_emoji`, `tool_name`, `tool_detail`, `active_tools`, `is_busy`
- `ActiveTool` — `key: ToolKey`, `name`, `detail`
- `ToolKey::OpenCode` — `name`, `detail`
- `AgentStatus` — `started`, `running`, `waiting`, `permission`, `ended`

**Output `plugins/opencode-types.ts`:**
```ts
// Auto-generated — DO NOT EDIT
export interface AgentState {
  tmux_pane: string;
  updated_at: number;
  data: {
    type: "opencode";
    session_id?: string;
    status: AgentStatus;
    event_type?: string;
    event_emoji?: string;
    tool_name?: string;
    tool_detail?: string;
    active_tools: ActiveTool[];
    is_busy: boolean;
  };
}
export type AgentStatus = "started" | "running" | "waiting" | "permission" | "ended";
export interface ActiveTool { ... }
```

**Output `opencode-protocol.json`:**
- JSON Schema (draft-07) generated via `schemars` derives
- Committed to repo for CI comparison

**Usage:**
```sh
cargo run -- generate-contract
```

### B — Contract Validation (`opencode-protocol.json`)

The JSON Schema file serves as a human-readable contract reference and enables CI verification. It is NOT used at runtime — only at test time to verify the committed file matches the canonical Rust structs.

The CI test `test_contract_is_up_to_date`:
1. Generates JSON Schema from current Rust types via `schemars`
2. Reads `opencode-protocol.json` from disk
3. Asserts they are identical
4. Also regenerates TypeScript types via `gen_contract` and compares

### C — Integration Tests

Existing tests already cover most of the IPC path. The key additions:

| Test | What it verifies |
|---|---|
| `test_contract_is_up_to_date` | Generated schema/types match committed files |
| `test_opencode_agent_state_roundtrip` | Full JSON → deserialize → serialize roundtrip |
| `test_stale_opencode_state_cleaned_up` | pane command changed, stale state removed |
| `test_opencode_state_retained_during_tool` | pane command = bash, but title = "OC \| ...", state kept |
| `test_stale_claude_state_cleaned_up` | Same for Claude agents |

Full list: 200+ tests covering all IPC message variants, merge logic, detection, stale cleanup, and UI rendering.

## File Changes

| File | Change |
|---|---|
| `src/bin/gen_contract.rs` | **New** — code generator binary |
| `plugins/opencode-types.ts` | **New** — generated TypeScript interfaces |
| `opencode-protocol.json` | **New** — generated JSON Schema |
| `Cargo.toml` | Add `schemars` dependency, add `[[bin]]` for gen_contract |
| `plugins/chikuwa.ts` | Import `opencode-types.ts` instead of manual interfaces |
| `src/agent/state.rs` | Add `#[derive(JsonSchema)]` to relevant types |
| `src/agent/opencode_state.rs` | Add `#[derive(JsonSchema)]` |

## IPC Event Contract

All recognized event types and their expected state transitions:

| Event type | Status | Active tools | Notes |
|---|---|---|---|
| `session.created` | Started | empty | New session |
| `session.status` (busy) | Started | preserved | Session active |
| `session.status` (idle) | Waiting | preserved | Session paused |
| `session.idle` | Waiting | cleared | Idle, clear tools |
| `session.deleted` | Ended | cleared | Session gone |
| `session.error` | Waiting | preserved | Error state |
| `tool.execute` | Running | append tool | Tool about to execute |
| `tool.running` | Running | append tool | Tool now running |
| `tool.completed` | Running | remove tool | Tool done |
| `tool.error` | Running | remove tool | Tool failed |
| `file.edited` | Running | preserved | File change |
| `permission.asked` | Permission | preserved | Waiting for user |
| `permission.replied` | Running | preserved | User responded |
| `command.executed` | Running | preserved | Shell command ran |

## Non-Goals

- Runtime validation (Zod, deny_unknown_fields) — all validation at test time
- Generic code generation from proc macros — hand-written generator for the 5 structs
- Cross-language type generation beyond TypeScript (no Python, no Go)
