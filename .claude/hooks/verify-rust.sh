#!/usr/bin/env bash
# Stop hook: run cargo check on crates with modified .rs files.
# Only checks crates that were actually changed in this session.

set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$WORKSPACE_ROOT"

# Find crates with modified .rs files (staged + unstaged vs HEAD)
changed_crates=$( \
    { git diff --name-only HEAD 2>/dev/null; git diff --name-only 2>/dev/null; } | \
    grep '\.rs$' | \
    grep '^crates/' | \
    cut -d'/' -f2 | \
    sort -u)

if [[ -z "$changed_crates" ]]; then
    exit 0  # No Rust files changed — skip
fi

# Build -p flags for each changed crate
pkg_flags=""
for crate in $changed_crates; do
    pkg_flags="$pkg_flags -p nebula-$crate"
done

if ! cargo check $pkg_flags --quiet 2>&1; then
    echo "❌ cargo check failed for changed crates: $changed_crates" >&2
    echo "Fix compilation errors before completing the session." >&2
    exit 2
fi

exit 0
