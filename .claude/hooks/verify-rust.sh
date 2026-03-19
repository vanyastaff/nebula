#!/usr/bin/env bash
# Stop hook: cargo check + nextest on crates with .rs files staged for commit.
# Only checks crates with STAGED changes — ignores unstaged work-in-progress
# from other sessions/agents.

set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$WORKSPACE_ROOT"

# Find crates with STAGED .rs files only (git diff --cached)
changed_crates=$( \
    git diff --cached --name-only 2>/dev/null | \
    grep '\.rs$' | \
    grep '^crates/' | \
    cut -d'/' -f2 | \
    sort -u)

if [[ -z "$changed_crates" ]]; then
    exit 0  # No staged Rust files — skip
fi

# Build -p flags for each changed crate
pkg_flags=""
for crate in $changed_crates; do
    pkg_flags="$pkg_flags -p nebula-$crate"
done

# Step 1: check compilation
if ! cargo check $pkg_flags --quiet 2>&1; then
    echo "❌ cargo check failed for: $changed_crates" >&2
    echo "Fix compilation errors before completing the session." >&2
    exit 2
fi

# Step 2: run tests via nextest (faster than cargo test)
if command -v cargo-nextest &>/dev/null; then
    if ! cargo nextest run $pkg_flags --no-fail-fast 2>&1; then
        echo "❌ Tests failed for: $changed_crates" >&2
        echo "Fix failing tests before completing the session." >&2
        exit 2
    fi
else
    if ! cargo test $pkg_flags --quiet 2>&1; then
        echo "❌ Tests failed for: $changed_crates" >&2
        echo "Fix failing tests before completing the session." >&2
        exit 2
    fi
fi

exit 0
