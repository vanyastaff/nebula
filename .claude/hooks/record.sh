#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.tool_name')" = "Bash" ] || allow
have_jq || allow
cmd="$(jqg '.tool_input.command')"
# Verified harness facts: PostToolUse fires only for exit-0 Bash; tool_response
# is a structured object. Trust the authenticated status; string-shape
# fallback treats a failure token as not-clean. Non-clean => not recorded.
ec="$(jqg '.tool_response.exit_code')"
[ -n "$ec" ] && [ "$ec" != "0" ] && allow
[ "$(jqg '.tool_response.success')" = "false" ] && allow
sresp="$(jqg '.tool_response')"
case "$sresp" in
  *'"exit_code"'*|*'"success"'*) : ;;
  *error*|*FAILED*|*"warning:"*|*"test result: FAILED"*) allow ;;
esac
# Record green ONLY for a CANONICAL CLEAN gate invocation — an ALLOWLIST of the
# exact clean shape, not a blocklist of evasions. Any chaining/masking/
# redirect/comment, any lint suppression, or a non-cargo/task argv0 => not
# recognized => not recorded => C blocks (fail-safe; agent runs gate plainly).
# Closes echo/||true/2>/dev/null/--cap-lints/RUSTFLAGS/multi-p/grep-of-docs.
case "$cmd" in
  *'||'*|*'&&'*|*';'*|*'|'*|*'`'*|*'$('*|*'>'*|*'<'*|*'#'*|*$'\n'*|*$'\r'*|*$'\t'*) allow ;;
  *' -A'*|*'--allow'*|*'--cap-lints'*|*'RUSTFLAGS='*) allow ;;
esac
core="$(printf '%s' "$cmd" | sed -E 's/^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*[[:space:]]+)*//')"
is_gate=0
if   [[ "$core" =~ ^cargo([[:space:]]+\+[^[:space:]]+)?[[:space:]]+clippy([[:space:]]|$) ]] && [[ "$core" =~ (^|[[:space:]])-D[[:space:]]+warnings([[:space:]]|$) ]]; then is_gate=1
elif [[ "$core" =~ ^cargo([[:space:]]+\+[^[:space:]]+)?[[:space:]]+nextest[[:space:]]+run([[:space:]]|$) ]]; then is_gate=1
elif [[ "$core" =~ ^task[[:space:]]+dev:check([[:space:]]|$) ]]; then is_gate=2
fi
[ "$is_gate" = 0 ] && allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"; st="$(load_state "$p")"
if [ "$is_gate" = 2 ]; then
  st="$(printf '%s' "$st" | jq -c '.gate_green = (.gate_green + ["*workspace*"] | unique)')"
else
  crate="$(printf '%s' "$core" | grep -oE -- '-p[[:space:]]+(nebula-)?[A-Za-z0-9_-]+' | head -1 | sed -E 's/^-p[[:space:]]+(nebula-)?//')"
  [ -n "$crate" ] && st="$(printf '%s' "$st" | jq -c --arg c "$crate" '.gate_green = (.gate_green + [$c] | unique)')"
fi
save_state "$p" "$st"
allow
