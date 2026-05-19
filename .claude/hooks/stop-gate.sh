#!/usr/bin/env bash
# D11: touched-crate set from git GROUND TRUTH (not solely B's recording).
# `git` here is read-only and triggers no tools — Stop-hook-safe. Over-
# detection is the safe direction (more crates must be green, never fewer).
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.stop_hook_active')" = "true" ] && allow   # loop guard (deadlock-safe)
have_jq || allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
st="$(load_state "$(turn_state_path "$sid" "$cwd")")"
printf '%s' "$st" | jq -e '.gate_green | index("*workspace*")' >/dev/null 2>&1 && allow
declare -A touched=()
_consider() {  # $1=path -> record its crate if it is a crate src .rs
  local p="${1%$'\r'}"   # strip trailing CR (Windows git-bash jq/git emit CRLF)
  printf '%s' "$p" | tr '\\' '/' | grep -qE '(^|/)crates/[^/]+/src/.*\.rs$' || return 0
  local c; c="$(crate_of "$p")"; [ -n "$c" ] && touched[$c]=1
}
# git ground truth: NUL-delimited, UNQUOTED paths (-z) — no quoting/sed-arrow
# pitfalls. Rename/Copy records are `XY new\0old`; gate BOTH paths + deletions.
while IFS= read -r -d '' rec; do
  [ -n "$rec" ] || continue
  xy="${rec:0:2}"; pth="${rec:3}"
  case "$xy" in
    R*|C*) _consider "$pth"; IFS= read -r -d '' old && _consider "$old" ;;
    *)     _consider "$pth" ;;
  esac
done < <(git -C "$cwd" status --porcelain -z -u 2>/dev/null)
# Spec §4.C 3rd source: changes COMMITTED this turn (git status goes clean
# after a commit; turn-state isn't reset by commit). turn_base = HEAD at A0;
# effective_turn_base repins it if a rebase/squash-merge orphaned that SHA so
# the diff stays scoped to THIS turn, not the whole rebased-in upstream delta.
tb="$(printf '%s' "$st" | jq -r '.turn_base // empty' 2>/dev/null)"
tb="$(effective_turn_base "$cwd" "$tb")"
if [ -n "$tb" ]; then
  while IFS= read -r -d '' f; do [ -n "$f" ] && _consider "$f"; done \
    < <(git -C "$cwd" diff --name-only -z "$tb"..HEAD 2>/dev/null)
fi
# corroborating B-union (turn-state recording — NEVER git-only; D11/constraint 1)
while IFS= read -r f; do [ -n "$f" ] && _consider "$f"; done < <(printf '%s' "$st" | jq -r '.impl_files_edited[]?' 2>/dev/null)
(( ${#touched[@]} == 0 )) && allow
missing=""
for c in "${!touched[@]}"; do
  printf '%s' "$st" | jq -e --arg c "$c" '.gate_green | index($c)' >/dev/null 2>&1 || missing="$missing $c"
done
[ -z "$missing" ] && allow
deny "You changed crate(s)$missing but never showed a clean clippy + nextest green for them. Run \`cargo clippy -p nebula-<crate> -- -D warnings\` and \`cargo nextest run -p nebula-<crate>\` (or \`task dev:check\`) before claiming done. (Touched set = git diff ground truth; weakening tests cannot help — A2 records green only for a clean gate, CI re-runs.)"
