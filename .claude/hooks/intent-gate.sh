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
    # `+ ` (space sentinel) not `+`: a source line that itself starts with `+`
    # would become `++…` and be miscounted as a header by `^\+([^+]|$)`.
    sed 's/^/+ /' "$cwd/$uf" 2>/dev/null
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

# Duplicate public-symbol heuristic: a NEW `pub fn|struct|trait NAME` whose
# NAME already exists (same kind) elsewhere in crates/*/src — the "47 date
# formatters" pattern. Added lines via ig_added_lines (untracked included).
dup_symbol() {
  local added kind name hit
  added="$(ig_added_lines | grep -E '^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' || true)"
  [ -n "$added" ] || return 1
  while IFS= read -r line; do
    kind="$(printf '%s' "$line" | sed -E 's/^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait).*/\1/')"
    name="$(printf '%s' "$line" | sed -E 's/^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait)[[:space:]]+([A-Za-z_][A-Za-z0-9_]*).*/\2/')"
    [ -n "$name" ] || continue
    hit="$(grep -rEl --include='*.rs' "(^|[^A-Za-z_])pub[[:space:]]+$kind[[:space:]]+$name([^A-Za-z0-9_]|$)" "$cwd"/crates/*/src 2>/dev/null | wc -l | tr -d ' ')"
    [ "${hit:-0}" -ge 2 ] && { printf '%s %s' "$kind" "$name"; return 0; }
  done <<< "$added"
  return 1
}
if d="$(dup_symbol)" && ! budget_justified; then
  ig_bump
  ig_log block "duplicate-symbol"
  deny "New public \`$d\` collides with an existing workspace symbol of the same kind. Reuse the existing one (search crates/*/src), or add a \`// budget-justified: <reason>\` line if the duplication is intentional. (ADR-0083; agents that do not search the codebase re-implement existing utilities.)"
fi

# Large-blob proxy for per-fn complexity (clippy.toml too-many-lines = 100).
# Longest run of consecutive added lines within one file, consuming the
# shared ig_added_lines stream (untracked-aware, per-file via the `+++ `
# header reset — uniform with the net-LoC / dup-symbol consumers).
BLOB_CAP=100
longest_added_run() {
  ig_added_lines | awk '
      /^\+\+\+ /      { run=0; next }
      /^\+/           { run++; if (run>max) max=run; next }
      { run=0 }
      END             { print max+0 }'
}
blob="$(longest_added_run)"
if [ "${blob:-0}" -gt "$BLOB_CAP" ] && ! budget_justified; then
  ig_bump
  ig_log block "blob-over-cap"
  deny "Turn adds a $blob-line contiguous block in a single file (cap $BLOB_CAP, the clippy.toml too-many-lines threshold). Decompose into smaller functions, or add a \`// budget-justified: <reason>\` line for intentional generated/table code. (ADR-0083 structural-budget tier.)"
fi

# New-file budget (ToF is the 2nd strongest decay predictor). ls-files
# --others lists individual files even inside a brand-new directory (which
# `git status --porcelain` would collapse to the dir).
new_files() {
  { [ -n "$tb" ] && git -C "$cwd" diff --name-only --diff-filter=A "$tb"..HEAD 2>/dev/null; \
    git -C "$cwd" diff --name-only --diff-filter=A --cached 2>/dev/null; \
    git -C "$cwd" ls-files --others --exclude-standard 2>/dev/null; } \
  | grep -E "$CODE_RE" | sort -u | grep -c . || true
}
NF_CAP=5
nf="$(new_files)"
if [ "${nf:-0}" -gt "$NF_CAP" ] && ! budget_justified; then
  ig_bump
  ig_log block "new-file-over-cap"
  deny "Turn adds $nf new code files (cap $NF_CAP). Consolidate into existing modules, or add a \`// budget-justified: <reason>\` line. (ADR-0083; file-count predicts architectural decay.)"
fi

NET_CAP=400
if [ "$net" -gt "$NET_CAP" ] && ! budget_justified; then
  ig_bump
  ig_log block "net-loc-over-cap"
  deny "Turn net +$net LoC exceeds the structural budget ($NET_CAP). Split the change into reviewable commits, delete dead code, or add a \`// budget-justified: <reason>\` line to an intentional large addition (e.g. generated/table data). (ADR-0083 structural-budget tier; large diffs are a top review-rejection cause.)"
fi

ig_log allow "within-budget"
allow
