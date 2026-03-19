#!/usr/bin/env bash
# Stop hook: block completion if crate code changed but context wasn't updated.
# Receives JSON on stdin (ignored — we check git state directly).

set -euo pipefail

CLAUDE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_ROOT="$(dirname "$CLAUDE_DIR")"

# Get crates with uncommitted changes (staged + unstaged vs HEAD)
changed_crates=$(cd "$WORKSPACE_ROOT" && \
    { git diff --name-only HEAD 2>/dev/null; git diff --name-only 2>/dev/null; } | \
    grep '^crates/' | cut -d'/' -f2 | sort -u)

if [[ -z "$changed_crates" ]]; then
    exit 0  # No crate changes — proceed normally
fi

missing_updates=""
for crate in $changed_crates; do
    context_file="$CLAUDE_DIR/crates/${crate}.md"
    if [[ ! -f "$context_file" ]]; then
        missing_updates="${missing_updates}\n- .claude/crates/${crate}.md is MISSING (new crate needs a context file)"
    else
        # Check if context file was also touched this session
        context_changed=$(cd "$WORKSPACE_ROOT" && \
            { git diff --name-only HEAD 2>/dev/null; git diff --name-only 2>/dev/null; } | \
            grep -c "\.claude/crates/${crate}\.md" || true)
        if [[ "$context_changed" == "0" ]]; then
            missing_updates="${missing_updates}\n- .claude/crates/${crate}.md NOT updated (but crates/${crate}/ was modified)"
        fi
    fi
done

if [[ -n "$missing_updates" ]]; then
    printf "Context files need updating before completing:%b\n" "$missing_updates" >&2
    echo "" >&2
    echo "Update each listed .claude/crates/{name}.md if invariants, decisions, or traps changed." >&2
    echo "If only implementation details changed (no architectural impact), add '<!-- reviewed: $(date +%Y-%m-%d) -->' at the bottom of the file." >&2
    exit 2  # Block completion — stderr is fed back to Claude as feedback
fi

exit 0
