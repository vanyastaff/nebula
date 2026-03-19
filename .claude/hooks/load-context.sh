#!/usr/bin/env bash
# SessionStart hook: inject workspace context into Claude's conversation.
# stdout from this hook is added as context Claude can see at session start.

set -euo pipefail

CLAUDE_DIR="$(cd "$(dirname "$0")/.." && pwd)"

echo "=== WORKSPACE CONTEXT (auto-loaded) ==="
echo ""

if [[ -f "$CLAUDE_DIR/ROOT.md" ]]; then
    cat "$CLAUDE_DIR/ROOT.md"
    echo ""
fi

if [[ -f "$CLAUDE_DIR/active-work.md" ]]; then
    cat "$CLAUDE_DIR/active-work.md"
    echo ""
fi

if [[ -f "$CLAUDE_DIR/pitfalls.md" ]]; then
    cat "$CLAUDE_DIR/pitfalls.md"
    echo ""
fi

echo "=== For crate details: read .claude/crates/{name}.md ==="

exit 0
