#!/usr/bin/env bash
# PreToolUse hook for the dx-tester subagent.
#
# dx-tester simulates a brand-new external SDK user. It must never dirty
# the main checkout with experimental code. This hook enforces that any
# Edit/Write target either:
#   1. lives outside the main worktree (i.e. in a separate git worktree), or
#   2. lives under <main>/target/dx-scratch/ inside the main checkout.
#
# Anything else is blocked with exit code 2 so the agent sees the reason.

set -euo pipefail

INPUT=$(cat)

# Extract file_path from the tool input (JSON on stdin).
FILE_PATH=""
if command -v python3 &>/dev/null; then
    FILE_PATH=$(printf '%s' "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(d.get('tool_input', {}).get('file_path', '') or '')
" 2>/dev/null || true)
else
    FILE_PATH=$(printf '%s' "$INPUT" \
        | grep -o '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' \
        | head -1 \
        | sed 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/' \
        || true)
fi

if [[ -z "$FILE_PATH" ]]; then
    # No file_path (e.g. Bash invocation) — nothing to check here.
    exit 0
fi

# Normalize Windows backslashes.
NORM_PATH=$(printf '%s' "$FILE_PATH" | tr '\\' '/')

# Primary worktree root = first entry in `git worktree list`.
MAIN_ROOT=$(git worktree list --porcelain 2>/dev/null \
    | awk '/^worktree /{print $2; exit}' \
    | tr '\\' '/' || true)

if [[ -z "$MAIN_ROOT" ]]; then
    echo "✗ dx-tester: refusing to write — not inside a git repository" >&2
    echo "  file: $FILE_PATH" >&2
    exit 2
fi

# Case-insensitive compare for Windows paths.
lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }
LC_MAIN=$(lower "$MAIN_ROOT")
LC_FILE=$(lower "$NORM_PATH")
LC_SCRATCH="${LC_MAIN}/target/dx-scratch"

# If the target is outside the main worktree, it's in a separate worktree. Allow.
case "$LC_FILE" in
    "$LC_MAIN"/*) ;;
    *) exit 0 ;;
esac

# Inside the main worktree — only target/dx-scratch/ is allowed.
case "$LC_FILE" in
    "$LC_SCRATCH"/*) exit 0 ;;
esac

echo "✗ dx-tester isolation violation: write to the main checkout is not allowed" >&2
echo "  file: $FILE_PATH" >&2
echo "  main: $MAIN_ROOT" >&2
echo "" >&2
echo "  Write into target/dx-scratch/ inside the main checkout, or run with" >&2
echo "  isolation:worktree so you're operating in a separate worktree." >&2
exit 2
