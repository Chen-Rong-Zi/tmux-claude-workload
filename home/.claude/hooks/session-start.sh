#!/usr/bin/env bash
# ============================================================================
# TEMPLATE — SessionStart hook: report Docker Sandboxes (sbx) status
# ============================================================================
# Minimal hook built around the OFF-THE-SHELF `sbx` (docker/tap/sbx) product.
# It does NOT implement any sandbox logic — it only:
#   1. checks that `sbx` is installed, its daemon is up, and you are signed in,
#   2. forwards the event to the legacy hooks (chikuwa tmux monitor, pixtuoid),
#   3. emits a JSON status so the user sees sbx state and the model learns to
#      use `sbx exec` via additionalContext.
#
# Wire into settings.json:
#   "SessionStart": [ { "hooks": [ {
#       "type": "command",
#       "command": "bash $HOME/.claude/hooks/session-start.sh",
#       "timeout": 60,
#       "statusMessage": "Checking Docker sandbox (sbx)..."
#   } ] } ]
# ============================================================================
set -uo pipefail

STDIN="$(cat 2>/dev/null || true)"

# Run a command with a hard timeout (macOS has no `timeout` binary).
quick() {
  local secs="$1"; shift
  local tmp; tmp="$(mktemp)"
  "$@" >"$tmp" 2>&1 &
  local pid=$!
  ( sleep "$secs"; kill -9 "$pid" 2>/dev/null ) &
  local killer=$!
  wait "$pid" 2>/dev/null; local rc=$?
  kill "$killer" 2>/dev/null
  cat "$tmp"; rm -f "$tmp"
  return "$rc"
}

# --- 1. sbx state ----------------------------------------------------------
SBX="$(command -v sbx || true)"
DAEMON_OUT=""; LS_OUT=""; READY=0; REASON=""

if [ -z "$SBX" ]; then
  REASON="sbx not installed (brew install docker/tap/sbx)"
else
  DAEMON_OUT="$(quick 5 sbx daemon status)"
  if echo "$DAEMON_OUT" | grep -qi "stopped"; then
    REASON="sbx daemon stopped (run: sbx daemon start)"
  else
    LS_OUT="$(quick 5 sbx ls --format json 2>&1)"
    if echo "$LS_OUT" | grep -qi "not signed in"; then
      REASON="sbx login required (run: sbx login)"
    else
      READY=1
    fi
  fi
fi

# --- 2. forward legacy hooks ------------------------------------------------
CHIKUWA="$(command -v chikuwa || true)"
PIXTUOID="$(command -v pixtuoid-hook || true)"
if [ -n "$CHIKUWA" ]; then
  printf '%s\n' "$STDIN" | tee -a /tmp/chikuwa.raw.log | chikuwa hook >/dev/null 2>&1 || true
fi
if [ -n "$PIXTUOID" ]; then
  printf '%s\n' "$STDIN" | pixtuoid-hook >/dev/null 2>&1 || true
fi

# --- 3. compose JSON ---------------------------------------------------------
if [ "$READY" = 1 ]; then
  COUNT="$(printf '%s' "$LS_OUT" | jq 'length' 2>/dev/null || echo 0)"
  system_message="Docker sandbox ready: sbx ($COUNT sandbox(es))"
else
  system_message="Docker sandbox NOT ready: $REASON"
fi

if [ "$READY" = 1 ]; then
  additional_context="Docker Sandboxes (sbx) is available this session.
- List sandboxes: sbx ls   (names for scripting: sbx ls -q)
- Run a command inside a sandbox: sbx exec <sandbox> <cmd>
- Open a shell: sbx exec -it <sandbox> bash
- Create one: sbx create claude <workspace-path>  (agents: claude, shell, ...)
Use sbx for isolated/stateful/heavy work instead of the host."
else
  additional_context="Docker Sandboxes (sbx) is installed but NOT ready: $REASON.
Until 'sbx login' and 'sbx daemon start' succeed, work on the host."
fi

jq -n \
  --arg sm "$system_message" \
  --arg ctx "$additional_context" \
  '{systemMessage:$sm, hookSpecificOutput:{hookEventName:"SessionStart", additionalContext:$ctx}}'
