#!/usr/bin/env bash
# PreToolUse hook: block obviously destructive shell commands.
# Receives JSON on stdin with tool_input.command.

set -euo pipefail

INPUT=$(cat)

CMD=""
if command -v python3 >/dev/null 2>&1; then
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
  exit 0
fi

# Block destructive git operations even if deny-list misses a variant.
if printf '%s' "$CMD" | grep -qE 'git.*(push.*--force|push.*-f([^a-zA-Z]|$)|reset --hard|clean -[a-zA-Z]*f|branch -D)'; then
  echo "Blocked by guard-bash.sh: destructive git operation detected." >&2
  exit 2
fi

# Block obvious writes to system paths.
if printf '%s' "$CMD" | grep -qE '(>|tee).*/(etc|usr|boot|System|Windows)/'; then
  echo "Blocked by guard-bash.sh: write to system path detected." >&2
  exit 2
fi

exit 0
