#!/bin/bash
# Create all GitHub issues for technical debt
# Run this script after gh CLI is available in PATH

set -e  # Exit on error

echo "Creating GitHub issues for technical debt..."
echo ""

# Check if gh is available
if ! command -v gh &> /dev/null; then
    echo "Error: gh CLI is not available in PATH"
    echo "Please restart your terminal or add gh to PATH"
    exit 1
fi

# Check if we're in a git repository
if ! git rev-parse --is-inside-work-tree > /dev/null 2>&1; then
    echo "Error: Not in a git repository"
    exit 1
fi

# Navigate to repository root
cd "$(git rev-parse --show-toplevel)"

echo "Repository: $(git remote get-url origin 2>/dev/null || echo 'No remote configured')"
echo ""

# Function to create issue
create_issue() {
    local num=$1
    local title=$2
    local priority=$3
    local milestone=$4
    local labels=$5
    local file=$6

    echo "Creating Issue #${num}: ${title}"

    gh issue create \
        --title "${title}" \
        --label "${labels}" \
        --milestone "${milestone}" \
        --body-file "${file}" || {
        echo "Failed to create issue #${num}"
        return 1
    }

    echo "✓ Created issue #${num}"
    echo ""
    sleep 1  # Rate limiting
}

# High Priority Issues (Sprint 5)
echo "=== HIGH PRIORITY ISSUES (Sprint 5) ==="
echo ""

create_issue 1 \
    "[HIGH] Rewrite nebula-parameter display system for nebula-validator compatibility" \
    "HIGH" \
    "Sprint 5" \
    "refactor,high-priority,nebula-parameter" \
    "docs/github_issues/issue_01_parameter_display_rewrite.md"

create_issue 2 \
    "[HIGH] Fix nebula-memory cache policy integration issues" \
    "HIGH" \
    "Sprint 5" \
    "bug,high-priority,nebula-memory" \
    "docs/github_issues/issue_02_memory_cache_policies.md"

create_issue 3 \
    "[HIGH] Implement pool maintenance and shutdown for nebula-resource" \
    "HIGH" \
    "Sprint 5" \
    "feature,high-priority,nebula-resource" \
    "docs/github_issues/issue_03_resource_pool_management.md"

create_issue 4 \
    "[HIGH] Implement ArenaGuard for RAII-based arena scope management" \
    "HIGH" \
    "Sprint 5" \
    "feature,high-priority,nebula-memory" \
    "docs/github_issues/issue_04_arena_scope_guard.md"

create_issue 7 \
    "[HIGH] Complete ResourceInstance test implementation with todo!() macros" \
    "HIGH" \
    "Sprint 5" \
    "bug,high-priority,nebula-resource,testing" \
    "docs/github_issues/issue_07_test_instance_todos.md"

# Medium Priority Issues (Sprint 6)
echo "=== MEDIUM PRIORITY ISSUES (Sprint 6) ==="
echo ""

create_issue 5 \
    "[MEDIUM] Re-enable and complete disabled modules across codebase" \
    "MEDIUM" \
    "Sprint 6" \
    "feature,medium-priority,technical-debt" \
    "docs/github_issues/issue_05_disabled_modules.md"

create_issue 6 \
    "[MEDIUM] Clean up excessive dead_code allows and unused functionality" \
    "MEDIUM" \
    "Sprint 6" \
    "refactor,medium-priority,code-quality" \
    "docs/github_issues/issue_06_dead_code_cleanup.md"

create_issue 8 \
    "[MEDIUM] Comprehensive unsafe code audit and documentation" \
    "MEDIUM" \
    "Sprint 6" \
    "security,medium-priority,nebula-memory,documentation" \
    "docs/github_issues/issue_08_unsafe_code_audit.md"

# Medium Priority Issues (Sprint 7)
echo "=== MEDIUM PRIORITY ISSUES (Sprint 7) ==="
echo ""

create_issue 9 \
    "[MEDIUM] Improve test coverage across all crates" \
    "MEDIUM" \
    "Sprint 7" \
    "testing,medium-priority,quality" \
    "docs/github_issues/issue_09_test_coverage.md"

echo ""
echo "=== SUMMARY ==="
echo "✓ All 9 issues created successfully!"
echo ""
echo "Next steps:"
echo "1. Review created issues on GitHub"
echo "2. Assign issues to team members"
echo "3. Set up project board for tracking"
echo "4. Update TODO comments in code with issue numbers"
echo "5. Begin Sprint 5 development"
echo ""
echo "View issues: gh issue list"
