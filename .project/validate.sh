#!/usr/bin/env bash
# Validates .project/context/ files stay within token budgets.
# Run: bash .project/validate.sh
#
# Project context lives under .project/context/ (moved out of .claude/
# on 2026-04-11 to avoid autopilot permission churn on every edit).

set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
CONTEXT_DIR="$PROJECT_DIR/context"
WORKSPACE_ROOT="$(dirname "$PROJECT_DIR")"
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

# Check core context files
check_budget "$CONTEXT_DIR/ROOT.md" 300 "context/ROOT.md"
check_budget "$CONTEXT_DIR/decisions.md" 500 "context/decisions.md"
check_budget "$CONTEXT_DIR/pitfalls.md" 300 "context/pitfalls.md"
check_budget "$CONTEXT_DIR/active-work.md" 200 "context/active-work.md"

# Check that every workspace crate has a context file.
# Iterate filesystem directly instead of `cargo metadata | xargs dirname`
# so Windows paths with drive letters (`C:\…`) don't break xargs.
if [[ -d "$WORKSPACE_ROOT/crates" ]]; then
    for crate_dir in "$WORKSPACE_ROOT"/crates/*/; do
        crate_name=$(basename "$crate_dir")
        # Skip crates that are actually proc-macro subdirectories inside a
        # parent crate (e.g. crates/action/macros). They share context.
        if [[ -f "$crate_dir/Cargo.toml" ]]; then
            check_budget "$CONTEXT_DIR/crates/${crate_name}.md" 500 "context/crates/${crate_name}.md"
        fi
    done
fi

echo ""
if [[ $ERRORS -gt 0 ]]; then
    echo "Found $ERRORS issue(s). Fix before committing."
    exit 1
else
    echo "All context files OK."
fi
