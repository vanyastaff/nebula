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

# Pre-filter: on the MAIN-thread Stop, stop-gate.sh (C) runs before us and
# owns broken-code turns — defer to it to avoid a duplicate deny. On
# SubagentStop (implement-worker) C is NOT wired, so we must enforce the
# budget here instead of deferring. `agent_id` is present only inside a
# subagent call (Claude Code hook input contract).
aid="$(jqg '.agent_id')"
impl_n="$(printf '%s' "$st" | jq -r '.impl_files_edited | length' 2>/dev/null || echo 0)"
green_n="$(printf '%s' "$st" | jq -r '.gate_green | length' 2>/dev/null || echo 0)"
if [ -z "$aid" ] && [ "${impl_n:-0}" -gt 0 ] && [ "${green_n:-0}" -eq 0 ]; then
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
# ground truth. Code files only. effective_turn_base repins a turn_base that a
# rebase/squash-merge orphaned (else net-LoC / new-file / blob / dup all count
# the whole rebased-in upstream delta AND any pre-turn branch commits that the
# rebase replayed — patch-ids stored at A0 locate the rewritten turn_base on
# the new line so the budget stays scoped to THIS turn).
tb="$(printf '%s' "$st" | jq -r '.turn_base // empty' 2>/dev/null)"
tb="$(printf '%s' "$st" | jq -r '.turn_base_patch_ids[]?' 2>/dev/null | effective_turn_base "$cwd" "$tb")"
CODE_RE='\.(rs|toml|sh|md)$'

# Unified added-content stream: a `+++ <path>` header per file then each added
# line prefixed `+`. Tracked deltas from `git diff --unified=0`; every
# untracked code file is wholly added. blob / dup / budget all consume this.
CODE_PS=(-- '*.rs' '*.toml' '*.sh' '*.md')   # pathspec mirror of CODE_RE
ig_added_lines() {
  { [ -n "$tb" ] && git -C "$cwd" diff --unified=0 "$tb"..HEAD "${CODE_PS[@]}" 2>/dev/null; \
    git -C "$cwd" diff --unified=0 "${CODE_PS[@]}" 2>/dev/null; \
    git -C "$cwd" diff --unified=0 --cached "${CODE_PS[@]}" 2>/dev/null; } \
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

# Path-based exempts for net-LoC / NF / blob: criterion benches are
# table-driven, SQL migrations are DDL fixtures, and snapshot / golden test
# data are inherently bulky — the path encodes the semantics, so a turn
# scoped to those paths does not consume the structural budget. (Bench
# files still respect a per-file blob cap (criterion files can be long but
# a single function should not be); see blob_cap_for_file below.)
# Agents cannot game the path because it is checked literally and a
# reviewer catches misplacement. (ADR-0083 escape-hatch hardening.)
is_exempt_path() {
  case "$1" in
    */benches/*.rs)                                       return 0 ;;
    */migrations/*.sql)                                   return 0 ;;
    */tests/golden/*|*/tests/snapshots/*|*/snapshots/*)   return 0 ;;
    *)                                                    return 1 ;;
  esac
}
# @generated convention (prettier, prost-build, tonic, etc.): a tool-emitted
# marker on the file's first few lines auto-exempts the file from the
# structural budget. Standard enough that catching it removes a class of
# false positives without giving agents a new game — the file would have to
# actually look generated.
is_generated_file() {
  local p="$cwd/$1"
  [ -f "$p" ] || return 1
  head -n 5 "$p" 2>/dev/null | grep -qE '@generated'
}
# Per-file added-line counts, consuming the shared ig_added_lines stream
# (untracked-aware, per-file via the `+++ ` header reset). Git diff emits
# `+++ b/<path>` for tracked changes; untracked files have no prefix. Strip
# the optional `a/` or `b/` so callers see a single canonical relative path
# (path-glob exemptions still match either form, but `is_generated_file`
# needs the on-disk path to be openable).
added_lines_per_file() {
  ig_added_lines | awk '
    function flush() { if (file != "") print file "\t" added+0 }
    /^\+\+\+ /      { flush(); file=$0; sub(/^\+\+\+[ \t]+/, "", file); sub(/^[ab]\//, "", file); added=0; next }
    /^\+/           { added++; next }
    END             { flush() }'
}

# net = added − deleted. `added` counts only NON-exempt, non-@generated
# files (path exemption applies symmetrically to net-LoC and the blob
# check below). `deleted` is raw numstat — exempt-file deletions still
# count, but net-negative is always allowed so that's fine.
added=0
while IFS=$'\t' read -r f a; do
  [ -n "$f" ] || continue
  is_exempt_path "$f" && continue
  is_generated_file "$f" && continue
  added=$((added + a))
done < <(added_lines_per_file)
deleted=0
while read -r _a d _; do
  [[ "$d" =~ ^[0-9]+$ ]] && deleted=$((deleted + d))
done < <( { [ -n "$tb" ] && git -C "$cwd" diff --numstat "$tb"..HEAD "${CODE_PS[@]}" 2>/dev/null; \
            git -C "$cwd" diff --numstat "${CODE_PS[@]}" 2>/dev/null; \
            git -C "$cwd" diff --numstat --cached "${CODE_PS[@]}" 2>/dev/null; } \
          | grep -E "$CODE_RE" || true )
net=$((added - deleted))

# Net-negative (cleanup / deletion) is always allowed — positive constraint.
if [ "$net" -lt 0 ]; then ig_log allow "net-negative"; allow; fi

# Escape token: `// budget-justified:` on any added line this turn.
#
# Drain-safe: `grep -q` exits on first match, triggering SIGPIPE on the
# `ig_added_lines` writer side (the `while-read | sed` loop and the
# three-way diff). Under `set -uo pipefail` that propagates rc=141 from
# the producer and `budget_justified` returns non-zero — the marker
# silently fails to escape. Use `grep -c` so the consumer drains the
# entire stream and producers exit cleanly.
#
# Anchored to start-of-line (`^\+[ \t]*//[ \t]*budget-justified:`) so that
# self-references — strings, deny-message bodies, comments-about-the-marker
# in this very file or the hook-test fixtures — do NOT count as markers.
# Only a real `// budget-justified: …` comment on an added line counts.
MARKER_RE='^\+[ \t]*//[ \t]*budget-justified:'
markers_count() {
  ig_added_lines | grep -cE "$MARKER_RE" | awk '{print $1+0}'
}
# Justification-quality heuristic: the text after `budget-justified:` must
# be at least 30 chars and mention one of the legitimate-bulk keywords.
# Catches lazy `// budget-justified: ok` / `// budget-justified: legacy`
# escapes without blocking genuine generated/table/criterion/migration/
# fixture/schema/snapshot/golden/test-data additions. Used by the blob
# check only — NF / net-LoC / dup checks still treat any marker as present.
markers_quality_count() {
  ig_added_lines \
    | grep -oE "$MARKER_RE"'.*' \
    | sed -E 's|^\+[ \t]*//[ \t]*budget-justified:[ \t]*||' \
    | awk '{ l = tolower($0); if (length($0) >= 30 && l ~ /table|generated|criterion|migration|fixture|schema|snapshot|golden|test[ \t]+data/) n++ } END { print n+0 }'
}
budget_justified()         { local n; n="$(markers_count)";         [ "${n:-0}" -gt 0 ]; }
budget_justified_quality() { local n; n="$(markers_quality_count)"; [ "${n:-0}" -gt 0 ]; }

# Per-turn marker budget. The `// budget-justified:` escape is for rare,
# intentional large additions; spamming it across files defeats the point.
# The cap runs BEFORE any blob/NF/net-LoC check so markers cannot authorize
# themselves — a 3-marker turn fails this gate regardless of what else is
# justified. (ADR-0083 escape-hatch hardening.)
MARKER_BUDGET=2
n_marks="$(markers_count)"
if [ "${n_marks:-0}" -gt "$MARKER_BUDGET" ]; then
  ig_bump
  ig_log block "marker-budget-exhausted"
  deny "Turn uses $n_marks \`// budget-justified:\` markers (cap $MARKER_BUDGET/turn). The escape is for rare intentional large additions; spamming it across files defeats the point. Consolidate the change or split into separate turns. (ADR-0083 escape-hatch hardening.)"
fi

# Duplicate public-symbol heuristic: a NEW `pub fn|struct|trait NAME` whose
# NAME already exists (same kind) elsewhere in crates/*/src — the "47 date
# formatters" pattern. Added lines via ig_added_lines (untracked included).
# Idiomatic, legitimately-repeated names (constructors + trait/accessor
# boilerplate) are skipped: they saturate any Rust workspace (`pub fn new`
# is in hundreds of files) and are never the duplicate-utility smell, so
# flagging them would invert the gate's signal. `pub async fn` is out of
# scope by design (the smell is plain `pub fn`); widening it would enlarge
# the false-positive surface this skiplist exists to contain.
dup_symbol() {
  local added kind name hit
  added="$(ig_added_lines | grep -E '^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' || true)"
  [ -n "$added" ] || return 1
  while IFS= read -r line; do
    kind="$(printf '%s' "$line" | sed -E 's/^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait).*/\1/')"
    [ -n "$kind" ] || continue
    name="$(printf '%s' "$line" | sed -E 's/^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait)[[:space:]]+([A-Za-z_][A-Za-z0-9_]*).*/\2/')"
    [ -n "$name" ] || continue
    case "$name" in
      new|default|len|is_empty|build|builder|from|try_from|into|iter|iter_mut|next|poll|fmt|clone|eq|hash|drop|deref|get|set|id|name|kind|value) continue ;;
    esac
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
# Longest run of consecutive added lines per file, consuming the shared
# ig_added_lines stream (untracked-aware, per-file via the `+++ ` header
# reset). Path-based caps replace the single BLOB_CAP=100: bench files are
# intrinsically criterion-table-driven, SQL migrations are DDL blobs, and
# snapshot / golden test fixtures are inherently bulky — the path encodes
# the semantics, so no marker is required there. (ADR-0083 escape-hatch
# hardening.) Random Rust source still capped at 100 — agent can't game
# the path because it is checked literally and reviewer catches misplacement.
blob_cap_for_file() {
  case "$1" in
    */benches/*.rs)                                       echo 300 ;;
    */migrations/*.sql)                                   echo 100000 ;;
    */tests/golden/*|*/tests/snapshots/*|*/snapshots/*)   echo 100000 ;;
    *)                                                    echo 100 ;;
  esac
}
longest_added_run_per_file() {
  ig_added_lines | awk '
    function flush() { if (file != "") print file "\t" max }
    /^\+\+\+ /      { flush(); file=$0; sub(/^\+\+\+[ \t]+/, "", file); sub(/^[ab]\//, "", file); run=0; max=0; next }
    /^\+/           { run++; if (run>max) max=run; next }
    { run=0 }
    END             { flush() }'
}
blob_path=""; blob_run=0; blob_cap=0
while IFS=$'\t' read -r bf br; do
  [ -n "$bf" ] || continue
  [ "${br:-0}" -gt 0 ] || continue
  is_generated_file "$bf" && continue
  bc="$(blob_cap_for_file "$bf")"
  if [ "$br" -gt "$bc" ]; then
    blob_path="$bf"; blob_run="$br"; blob_cap="$bc"; break
  fi
done < <(longest_added_run_per_file)
if [ -n "$blob_path" ] && ! budget_justified_quality; then
  ig_bump
  ig_log block "blob-over-cap"
  deny "Turn adds a $blob_run-line contiguous block in \`$blob_path\` (path-specific cap $blob_cap). Decompose into smaller functions, or add a \`// budget-justified: <reason>\` line (≥30 chars, mentioning table/generated/criterion/migration/fixture/schema/snapshot/golden/test data) for intentional generated/table code. Bench paths (\`*/benches/*.rs\`), SQL migrations, and snapshot/golden test fixtures are auto-exempt by path; files whose first lines carry \`@generated\` are also exempt. (ADR-0083 structural-budget tier; escape-hatch hardening.)"
fi

# New-file budget (ToF is the 2nd strongest decay predictor). ls-files
# --others lists individual files even inside a brand-new directory (which
# `git status --porcelain` would collapse to the dir).
new_files() {
  { [ -n "$tb" ] && git -C "$cwd" diff --name-only --diff-filter=A "$tb"..HEAD 2>/dev/null; \
    git -C "$cwd" diff --name-only --diff-filter=A --cached 2>/dev/null; \
    git -C "$cwd" ls-files --others --exclude-standard 2>/dev/null; } \
  | grep -E "$CODE_RE" | sort -u \
  | while IFS= read -r f; do
      is_exempt_path "$f" && continue
      is_generated_file "$f" && continue
      printf '%s\n' "$f"
    done | grep -c . || true
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
