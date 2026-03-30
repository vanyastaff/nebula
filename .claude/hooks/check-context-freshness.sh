#!/usr/bin/env bash
# Stop hook: block completion if crate code changed but context wasn't updated.
# Checks both uncommitted changes AND recent commits (since session start).

set -euo pipefail

CLAUDE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_ROOT="$(dirname "$CLAUDE_DIR")"
cd "$WORKSPACE_ROOT"

# Collect all changed files: uncommitted (staged + unstaged) + recent session commits.
# "Recent" = commits made today that haven't been pushed, approximating "this session".
all_changed_files=$(
    {
        # Uncommitted changes (staged + unstaged vs HEAD)
        git diff --name-only HEAD 2>/dev/null
        git diff --name-only 2>/dev/null
        # Commits ahead of remote (unpushed = likely this session)
        git diff --name-only "$(git merge-base HEAD @{u} 2>/dev/null || git rev-list --max-parents=0 HEAD | head -1)"..HEAD 2>/dev/null
    } | sort -u
)

# Get crates with changes
changed_crates=$(printf '%s\n' "$all_changed_files" | \
    grep '^crates/' | cut -d'/' -f2 | sort -u)

if [[ -z "$changed_crates" ]]; then
    exit 0  # No crate changes — proceed normally
fi

missing_updates=""
for crate in $changed_crates; do
    # Skip deleted crates — no context file needed
    if [[ ! -d "crates/${crate}" ]]; then
        continue
    fi
    context_file=".claude/crates/${crate}.md"
    if [[ ! -f "$context_file" ]]; then
        missing_updates="${missing_updates}\n- .claude/crates/${crate}.md is MISSING (new crate needs a context file)"
    else
        # Check if context file appears anywhere in all_changed_files
        context_changed=$(printf '%s\n' "$all_changed_files" | \
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
