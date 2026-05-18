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

# Turn diff scope: committed-this-turn (turn_base..HEAD) + working tree +
# staged + UNTRACKED. Agents typically leave new files unstaged, so a diff-
# only view misses them; stop-gate.sh (C) uses the same `git status -u`
# ground truth. Code files only.
tb="$(printf '%s' "$st" | jq -r '.turn_base // empty' 2>/dev/null)"
CODE_RE='\.(rs|toml|sh|md)$'

# Unified added-content stream: a `+++ <path>` header per file then each added
# line prefixed `+`. Tracked deltas from `git diff --unified=0`; every
# untracked code file is wholly added. blob / dup / budget all consume this.
ig_added_lines() {
  { [ -n "$tb" ] && git -C "$cwd" diff --unified=0 "$tb"..HEAD 2>/dev/null; \
    git -C "$cwd" diff --unified=0 2>/dev/null; \
    git -C "$cwd" diff --unified=0 --cached 2>/dev/null; } \
  | grep -E '^(\+\+\+ |\+)'
  while IFS= read -r uf; do
    [ -n "$uf" ] || continue
    printf '+++ %s\n' "$uf"
    sed 's/^/+/' "$cwd/$uf" 2>/dev/null
  done < <(git -C "$cwd" ls-files --others --exclude-standard 2>/dev/null \
            | grep -E "$CODE_RE" || true)
}

# net = added − deleted. added = stream added lines minus `+++ ` headers;
# deleted = numstat deletions on tracked changes (untracked delete nothing).
added="$(ig_added_lines | grep -cE '^\+([^+]|$)')"
deleted=0
while read -r _a d _; do
  [[ "$d" =~ ^[0-9]+$ ]] && deleted=$((deleted + d))
done < <( { [ -n "$tb" ] && git -C "$cwd" diff --numstat "$tb"..HEAD 2>/dev/null; \
            git -C "$cwd" diff --numstat 2>/dev/null; \
            git -C "$cwd" diff --numstat --cached 2>/dev/null; } \
          | grep -E "$CODE_RE" || true )
net=$((added - deleted))

# Net-negative (cleanup / deletion) is always allowed — positive constraint.
if [ "$net" -lt 0 ]; then ig_log allow "net-negative"; allow; fi

# Escape token: `// budget-justified:` on any added line this turn.
budget_justified() { ig_added_lines | grep -qE '//[[:space:]]*budget-justified:'; }

NET_CAP=400
if [ "$net" -gt "$NET_CAP" ] && ! budget_justified; then
  ig_bump
  ig_log block "net-loc-over-cap"
  deny "Turn net +$net LoC exceeds the structural budget ($NET_CAP). Split the change into reviewable commits, delete dead code, or add a \`// budget-justified: <reason>\` line to an intentional large addition (e.g. generated/table data). (ADR-0083 structural-budget tier; large diffs are a top review-rejection cause.)"
fi

ig_log allow "within-budget"
allow
