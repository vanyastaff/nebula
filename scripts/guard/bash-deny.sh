#!/usr/bin/env bash
# D10: NOT a security boundary — a cheap fail-OPEN advisory tripwire. The real
# no-cheat guarantee is B (edit-guard) + A2 (lint-suppression-aware recorder)
# + C (Stop-gate) + lefthook/CI. Any doubt (no jq / non-Bash / unreadable /
# obfuscated / ambiguous) => allow. No shell parser; substring only.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
have_jq || allow
[ "$(jqg '.tool_name')" = "Bash" ] || allow
c="$(jqg '.tool_input.command')"; [ -n "$c" ] || allow
g() { printf '%s' "$c" | grep -Eq "$1"; }
if g 'git[[:space:]]+commit' && g '(--no-verify|--no-gpg-sign|core\.hooksPath=)'; then
  deny "Don't bypass lefthook (--no-verify/--no-gpg-sign/core.hooksPath). Fix what it flags."
fi
if g '(^|[[:space:]])cargo([[:space:]]|$)' && g '(^|[[:space:]])fmt([[:space:]]|$)' && g '(^|[[:space:]])--all([[:space:]]|$)'; then
  deny "cargo fmt --all trips Windows os-error-206 / false green. Use bash scripts/pre-commit-fmt-check.sh or cargo fmt -p <crate>."
fi
if g 'git[[:space:]]+push' && g '(--force([[:space:]]|=|$)|--force-with-lease|(^|[[:space:]])-f([[:space:]]|$))' && [ "${NEBULA_ALLOW_FORCE:-}" != "1" ]; then
  deny "Force-push to shared history blocked (AGENTS.md). Set NEBULA_ALLOW_FORCE=1 to override."
fi
allow
