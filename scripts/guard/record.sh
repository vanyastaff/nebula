#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.tool_name')" = "Bash" ] || allow
have_jq || allow
cmd="$(jqg '.tool_input.command')"; resp="$(jqg '.tool_response')"
case "$resp" in *error*|*FAILED*|*"warning:"*|*"test result: FAILED"*) allow;; esac
# D10: a clippy run that SUPPRESSED lints is not a clean gate — refuse to
# record green (structural home of the old hook-A clippy rule). Substring is
# safe here: imperfect detection only ever fails toward "not recorded" (C then
# blocks), never toward a false green.
if printf '%s' "$cmd" | grep -Eq 'cargo[[:space:]].*clippy' \
   && printf '%s' "$cmd" | grep -Eq '([[:space:]]-A([[:space:]]|=|[A-Za-z])|--allow([[:space:]]|=)|RUSTFLAGS=[^&]*-A)'; then
  allow
fi
is_gate=0
[[ "$cmd" =~ cargo[[:space:]]+clippy.*-D ]] && is_gate=1
[[ "$cmd" =~ cargo[[:space:]]+nextest[[:space:]]+run ]] && is_gate=1
[[ "$cmd" =~ (^|[[:space:]])task[[:space:]]+dev:check ]] && is_gate=2
[ "$is_gate" = 0 ] && allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"; st="$(load_state "$p")"
if [ "$is_gate" = 2 ]; then
  st="$(printf '%s' "$st" | jq -c '.gate_green = (.gate_green + ["*workspace*"] | unique)')"
else
  crate="$(printf '%s' "$cmd" | sed -n 's/.*-p[[:space:]]\{1,\}\(nebula-\)\{0,1\}\([A-Za-z0-9_-]\{1,\}\).*/\2/p')"
  [ -n "$crate" ] && st="$(printf '%s' "$st" | jq -c --arg c "$crate" '.gate_green = (.gate_green + [$c] | unique)')"
fi
save_state "$p" "$st"
allow
