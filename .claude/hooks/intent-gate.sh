#!/usr/bin/env bash
# Layer-2 deterministic structural-budget gate (ADR-0083). Runs AFTER
# stop-gate.sh (C). Pure git+bash, no model. Blocking convention from _lib.sh:
# deny() => stderr + exit 2 (turn continues); allow() => exit 0.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.stop_hook_active')" = "true" ] && allow   # loop guard
have_jq || allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
TS_PATH="$(turn_state_path "$sid" "$cwd")"
st="$(load_state "$TS_PATH")"

# Audit log: every verdict. value-free reasons only.
ig_log() { # $1=verdict $2=reason
  local d f; d="$(git_common_dir "$cwd")/.nebula-guard"
  mkdir -p "$d" 2>/dev/null || return 0
  f="$d/intent-log-${sid:-unknown}.jsonl"
  printf '{"v":"%s","r":"%s"}\n' "$1" "$2" >>"$f" 2>/dev/null || true
}

# Loop counter lives in the RAW turn-state file (load_state projects it away;
# turn-reset.sh rewrites the file fresh at A0 so it is naturally per-turn).
ig_attempts() { have_jq || { echo 0; return 0; }; jq -r '.intent_attempts // 0' "$TS_PATH" 2>/dev/null || echo 0; }
ig_bump() {
  have_jq || return 0; [ -f "$TS_PATH" ] || return 0
  local t; t="$(jq -c '.intent_attempts = ((.intent_attempts // 0) + 1)' "$TS_PATH" 2>/dev/null)" \
    && printf '%s' "$t" >"$TS_PATH" 2>/dev/null || true
}

# Pre-filter: C (stop-gate) owns broken code. If the turn touched lib crates
# but recorded no green gate, C will block — do not double-judge.
impl_n="$(printf '%s' "$st" | jq -r '.impl_files_edited | length' 2>/dev/null || echo 0)"
green_n="$(printf '%s' "$st" | jq -r '.gate_green | length' 2>/dev/null || echo 0)"
if [ "${impl_n:-0}" -gt 0 ] && [ "${green_n:-0}" -eq 0 ]; then
  ig_log allow "c-owns-broken"; allow
fi

# Loop bound: after N=2 denies this turn, allow + log escalation (never trap
# the human; the log is the review surface).
attempts="$(ig_attempts)"
if [ "${attempts:-0}" -ge 2 ]; then
  ig_log escalate "loop-bound-after-2"; allow
fi

ig_log allow "skeleton-default"
allow
