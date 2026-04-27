#!/usr/bin/env bash
# PreToolUse hook: block obviously destructive shell commands.
# Receives JSON on stdin with tool_input.command.

set -euo pipefail

INPUT=$(cat)

CMD=""
# Prefer jq when available, then python3, then a basic sed fallback.
if command -v jq >/dev/null 2>&1; then
  CMD=$(printf '%s' "$INPUT" | jq -r '.tool_input.command // ""' 2>/dev/null || true)
fi

if [[ -z "$CMD" ]] && command -v python3 >/dev/null 2>&1; then
  CMD=$(printf '%s' "$INPUT" | python3 -c '
import json, sys
try:
    payload = json.load(sys.stdin)
except Exception:
    print("")
    raise SystemExit(0)
print(payload.get("tool_input", {}).get("command", "") or "")
' 2>/dev/null || true)
fi

if [[ -z "$CMD" ]]; then
  CMD=$(printf '%s' "$INPUT" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
fi

if [[ -z "$CMD" ]]; then
  exit 0
fi

# Block destructive git operations even if deny-list misses a variant.
# Note: `--force(\s|$)` (boundary) lets safe variants pass: `--force-with-lease`
# and `--force-if-includes` are Git's safe force-pushes (reject if remote
# changed since last fetch / since the local ref was last updated). The
# unsafe bare `--force` and short `-f` flags remain blocked.
if printf '%s' "$CMD" | grep -qE 'git.*(push.*--force(\s|$)|push.*-f([^a-zA-Z]|$)|reset --hard|clean -[a-zA-Z]*f)'; then
  echo "Blocked by guard-bash.sh: destructive git operation detected." >&2
  exit 2
fi

# Allow branch deletion workflows, but protect force-deletion of primary branches.
if printf '%s' "$CMD" | grep -qiE 'git[[:space:]].*branch[[:space:]]+-D[[:space:]]+(main|master|develop|dev)([[:space:];]|$)'; then
  echo "Blocked by guard-bash.sh: refusing force-delete of protected branch." >&2
  exit 2
fi

# Block obvious writes to system paths.
if printf '%s' "$CMD" | grep -qE '(>|tee).*/(etc|usr|boot|System|Windows)/'; then
  echo "Blocked by guard-bash.sh: write to system path detected." >&2
  exit 2
fi

exit 0
