#!/usr/bin/env bash
# PostToolUse hook: if a .claude/ file was just edited, check its token budget.
# Receives JSON on stdin with tool_input.file_path.

set -euo pipefail

INPUT=$(cat)

# Extract file_path — try python3 first, fall back to grep+sed
FILE_PATH=""
if command -v python3 &>/dev/null; then
    FILE_PATH=$(printf '%s' "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(d.get('tool_input', {}).get('file_path', '') or '')
" 2>/dev/null || true)
else
    FILE_PATH=$(printf '%s' "$INPUT" | grep -o '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/' || true)
fi

# Only care about .claude/ context files
if [[ -z "$FILE_PATH" ]] || ! printf '%s' "$FILE_PATH" | grep -q '\.claude/'; then
    exit 0
fi

# File must exist
if [[ ! -f "$FILE_PATH" ]]; then
    exit 0
fi

CHARS=$(wc -c < "$FILE_PATH" 2>/dev/null || echo "0")
APPROX_TOKENS=$((CHARS / 4))

# Determine budget by filename
BUDGET=500
if printf '%s' "$FILE_PATH" | grep -q 'ROOT\.md'; then
    BUDGET=300
elif printf '%s' "$FILE_PATH" | grep -q 'pitfalls\.md'; then
    BUDGET=300
elif printf '%s' "$FILE_PATH" | grep -q 'active-work\.md'; then
    BUDGET=200
fi

if [[ $APPROX_TOKENS -gt $BUDGET ]]; then
    echo "⚠ ${FILE_PATH} is ~${APPROX_TOKENS} tokens (budget: ${BUDGET}). Trim it — remove anything extractable from code." >&2
    exit 2  # Block and feed back to Claude
fi

exit 0
