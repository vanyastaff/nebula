#!/usr/bin/env bash
# PreToolUse hook: warn when editing nebula-core trait definitions.
# Receives JSON on stdin with tool_input.file_path.

set -euo pipefail

INPUT=$(cat)

# Extract file_path
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

# Only care about nebula-core source files
if [[ -z "$FILE_PATH" ]]; then
    exit 0
fi

# Normalize path separators for Windows
NORM_PATH=$(printf '%s' "$FILE_PATH" | tr '\\' '/')

# Check if this is a nebula-core source file
if ! printf '%s' "$NORM_PATH" | grep -q 'crates/core/src'; then
    exit 0
fi

# Check if the edit touches trait definitions
OLD_STRING=""
if command -v python3 &>/dev/null; then
    OLD_STRING=$(printf '%s' "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(d.get('tool_input', {}).get('old_string', '') or d.get('tool_input', {}).get('content', '') or '')
" 2>/dev/null || true)
fi

# Flag trait changes — these cascade to 25+ crates
if printf '%s' "$OLD_STRING" | grep -qE '^\s*(pub\s+)?(async\s+)?trait\s|^\s*fn\s.*\)\s*(->\s*|where\s|;|\{)'; then
    echo "⚠ Editing a trait in nebula-core — this cascades to 25+ dependent crates." >&2
    echo "Adding new ID types is safe; changing trait signatures is a breaking change." >&2
    echo "Proceed only if this was explicitly approved." >&2
    # exit 0 = warn but allow; change to exit 2 to block
    exit 0
fi

exit 0
