#!/usr/bin/env bash
# SessionStart hook: inject workspace context into Claude's conversation.
# stdout from this hook is added as context Claude can see at session start.
#
# Project context files live under .project/context/ (moved out of .claude/
# on 2026-04-11 to avoid autopilot permission churn on every edit).

set -euo pipefail

HOOK_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_ROOT="$(dirname "$HOOK_DIR")"
CONTEXT_DIR="$WORKSPACE_ROOT/.project/context"

echo "=== WORKSPACE CONTEXT (auto-loaded) ==="
echo ""

if [[ -f "$CONTEXT_DIR/ROOT.md" ]]; then
    cat "$CONTEXT_DIR/ROOT.md"
    echo ""
fi

if [[ -f "$CONTEXT_DIR/active-work.md" ]]; then
    cat "$CONTEXT_DIR/active-work.md"
    echo ""
fi

if [[ -f "$CONTEXT_DIR/pitfalls.md" ]]; then
    cat "$CONTEXT_DIR/pitfalls.md"
    echo ""
fi

echo "=== For crate details: read .project/context/crates/{name}.md ==="

exit 0