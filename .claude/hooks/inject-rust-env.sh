#!/usr/bin/env bash
# PreToolUse hook: inject Rust/Cargo/sccache env vars into every Bash command.
# Reads the command from stdin JSON, prepends env exports, outputs modified JSON.
# This is the canonical source of agent env (interactive shells should mirror
# what they need explicitly — see docs/dev-setup.md "Environment variables").

set -euo pipefail

INPUT=$(cat)

CMD=""
if command -v jq >/dev/null 2>&1; then
  CMD=$(printf '%s' "$INPUT" | jq -r '.tool_input.command // ""' 2>/dev/null || true)
fi

if [[ -z "$CMD" ]] && command -v python3 >/dev/null 2>&1; then
  CMD=$(printf '%s' "$INPUT" | python3 -c '
import json, sys
try:
    payload = json.load(sys.stdin)
    print(payload.get("tool_input", {}).get("command", "") or "")
except Exception:
    print("")
' 2>/dev/null || true)
fi

# No command found — allow unchanged.
if [[ -z "$CMD" ]]; then
  exit 0
fi

ENV_PREFIX=''
# --- sccache ----------------------------------------------------------------
# Shared compilation cache across worktrees.
# IMPORTANT: incremental must be OFF — it bypasses sccache and produces stale hits.
ENV_PREFIX+='export RUSTC_WRAPPER="sccache"; '
ENV_PREFIX+='export SCCACHE_CACHE_SIZE="50G"; '
ENV_PREFIX+='export CARGO_INCREMENTAL="0"; '
# --- cargo output -----------------------------------------------------------
# No ANSI — makes cargo/clippy output parseable by the agent
ENV_PREFIX+='export CARGO_TERM_COLOR="never"; '
# Retry transient network failures (crates.io, git deps)
ENV_PREFIX+='export CARGO_NET_RETRY="3"; '
# --- compile flags ----------------------------------------------------------
# Faster incremental linking on Linux (ignored if mold not installed)
ENV_PREFIX+='export CARGO_PROFILE_DEV_LTO="false"; '
# Forward errors immediately, don't batch — agent sees failures sooner
ENV_PREFIX+='export CARGO_PROFILE_DEV_CODEGEN_UNITS="256"; '
# --- rust runtime -----------------------------------------------------------
# Show backtraces on panics during tests/runs
ENV_PREFIX+='export RUST_BACKTRACE="1"; '
# Treat rustdoc warnings as errors (mirrors pre-push docs check)
ENV_PREFIX+='export RUSTDOCFLAGS="-D warnings"; '
# Suppress noisy library logs; agent output stays clean
ENV_PREFIX+='export RUST_LOG="warn"; '
# --- test runner ------------------------------------------------------------
# Nextest: fail-fast + no success noise (mirrors agent nextest profile)
ENV_PREFIX+='export NEXTEST_PROFILE="agent"; '
# --- locale -----------------------------------------------------------------
ENV_PREFIX+='export LANG="en_US.UTF-8"; export LC_ALL="en_US.UTF-8"; '

MODIFIED_CMD="${ENV_PREFIX}${CMD}"

if command -v jq >/dev/null 2>&1; then
  printf '%s' "$INPUT" | jq --arg cmd "$MODIFIED_CMD" '.tool_input.command = $cmd'
else
  # jq unavailable — allow without modification rather than blocking.
  exit 0
fi
