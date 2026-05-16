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
declare -A touched
while IFS= read -r f; do
  [ -n "$f" ] || continue
  c="$(crate_of "$f")"; [ -n "$c" ] && touched[$c]=1
done < <(
  { git -C "$cwd" status --porcelain -u 2>/dev/null | sed -E 's/^.{3}//; s/^.* -> //'
    printf '%s' "$st" | jq -r '.impl_files_edited[]?'
  } | tr '\\' '/' | grep -E '(^|/)crates/[^/]+/src/[^[:space:]]*\.rs$' )
[ "${#touched[@]}" -eq 0 ] && allow
missing=""
for c in "${!touched[@]}"; do
  printf '%s' "$st" | jq -e --arg c "$c" '.gate_green | index($c)' >/dev/null 2>&1 || missing="$missing $c"
done
[ -z "$missing" ] && allow
deny "You changed crate(s)$missing but never showed a clean clippy + nextest green for them. Run \`cargo clippy -p nebula-<crate> -- -D warnings\` and \`cargo nextest run -p nebula-<crate>\` (or \`task dev:check\`) before claiming done. (Touched set = git diff ground truth; weakening tests cannot help — A2 records green only for a clean gate, CI re-runs.)"
