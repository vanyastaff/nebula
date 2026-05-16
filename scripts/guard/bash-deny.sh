#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
have_jq || deny "jq is required by the bash guard and is missing (fail-closed). Install jq."
printf '%s' "$guard_input" | jq -e . >/dev/null 2>&1 || deny "Input is not valid JSON (fail-closed). Cannot verify the command."
[ "$(jqg '.tool_name')" = "Bash" ] || allow
cmd="$(jqg '.tool_input.command')"; [ -n "$cmd" ] || allow
# Match deny rules against the RESOLVED command (balanced quotes removed like
# the shell) so `--"no-verify"` / `ca"rg"o` cannot hide a violation, while
# benign quoted commands still resolve and pass.
raw="$(resolve_cmd "$cmd")"
[ "$raw" = "UNPARSEABLE" ] && deny "Command not safely verifiable (shell substitution/chaining, unbalanced quotes, or env --split-string). Run it as a single plain command."
argv0="$(normalize_argv0 "$cmd")"
[ "$argv0" = "UNPARSEABLE" ] && deny "Command not safely verifiable. Run it as a single plain command."
if [ "$argv0" = git ] && [[ "$raw" =~ (^|[[:space:]])commit([[:space:]]|$) ]] \
   && [[ "$raw" =~ (--no-verify|(^|[[:space:]])-n([[:space:]]|$)|--no-gpg-sign|core\.hooksPath=) ]]; then
  deny "Bypassing lefthook is the top-level cheat. Commit without --no-verify/-n/--no-gpg-sign; fix what the hook flags."
fi
if [ "$argv0" = cargo ] && [[ "$raw" =~ (^|[[:space:]])clippy([[:space:]]|$) ]] \
   && [[ "$raw" =~ ([[:space:]]-A[[:space:]]|--allow[[:space:]]|RUSTFLAGS=[^\&]*-A) ]]; then
  deny "Silencing clippy to reach green is cheating the oracle. Fix the lint or add a justified #[allow] in code."
fi
if [ "$argv0" = cargo ] && [[ "$raw" =~ (^|[[:space:]])fmt([[:space:]]|$) ]] \
   && [[ "$raw" =~ ([[:space:]]|^)--all([[:space:]]|$) ]]; then
  deny "cargo fmt --all trips Windows os-error-206 and false green. Use bash scripts/pre-commit-fmt-check.sh or cargo fmt -p <crate>."
fi
if [ "$argv0" = git ] && [[ "$raw" =~ (^|[[:space:]])push([[:space:]]|$) ]] \
   && [[ "$raw" =~ (--force([[:space:]]|=|$)|--force-with-lease|(^|[[:space:]])-f([[:space:]]|$)) ]] \
   && [ "${NEBULA_ALLOW_FORCE:-}" != "1" ]; then
  deny "Force-push to shared history is blocked (AGENTS.md). Set NEBULA_ALLOW_FORCE=1 only if you truly mean it."
fi
allow
