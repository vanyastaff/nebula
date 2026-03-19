#!/usr/bin/env bash
# Validates .claude/ context files stay within token budgets
# Run: bash .claude/validate.sh

set -euo pipefail

CLAUDE_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(dirname "$CLAUDE_DIR")"
ERRORS=0

# Approximate tokens ≈ chars / 4
check_budget() {
    local file="$1"
    local max_tokens="$2"
    local label="$3"

    if [[ ! -f "$file" ]]; then
        echo "⚠ MISSING: $label ($file)"
        ERRORS=$((ERRORS + 1))
        return
    fi

    local chars
    chars=$(wc -c < "$file")
    local approx_tokens=$((chars / 4))

    if [[ $approx_tokens -gt $max_tokens ]]; then
        echo "✗ OVER BUDGET: $label — ~${approx_tokens} tokens (max ${max_tokens})"
        ERRORS=$((ERRORS + 1))
    else
        echo "✓ $label — ~${approx_tokens}/${max_tokens} tokens"
    fi
}

# Check core files
check_budget "$CLAUDE_DIR/ROOT.md" 300 "ROOT.md"
check_budget "$CLAUDE_DIR/decisions.md" 500 "decisions.md"
check_budget "$CLAUDE_DIR/pitfalls.md" 300 "pitfalls.md"
check_budget "$CLAUDE_DIR/active-work.md" 200 "active-work.md"

# Check that every workspace crate has a context file
if [[ -f "$WORKSPACE_ROOT/Cargo.toml" ]]; then
    while IFS= read -r crate_path; do
        crate_name=$(basename "$crate_path")
        check_budget "$CLAUDE_DIR/crates/${crate_name}.md" 500 "crates/${crate_name}.md"
    done < <(cargo metadata --no-deps --format-version 1 2>/dev/null \
        | jq -r '.packages[].manifest_path' \
        | xargs -I{} dirname {} \
        | sort -u)
fi

echo ""
if [[ $ERRORS -gt 0 ]]; then
    echo "Found $ERRORS issue(s). Fix before committing."
    exit 1
else
    echo "All context files OK."
fi
