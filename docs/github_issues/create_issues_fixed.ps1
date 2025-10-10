# Create all GitHub issues for technical debt (without milestones)
# Run this script after gh CLI is available in PATH
# Usage: .\create_issues_fixed.ps1

$ErrorActionPreference = "Stop"

Write-Host "Creating GitHub issues for technical debt..." -ForegroundColor Cyan
Write-Host ""

# Check if gh is available
try {
    $null = Get-Command gh -ErrorAction Stop
} catch {
    Write-Host "Error: gh CLI is not available in PATH" -ForegroundColor Red
    Write-Host "Please restart your terminal or add gh to PATH" -ForegroundColor Yellow
    exit 1
}

# Check if we're in a git repository
try {
    $repoRoot = git rev-parse --show-toplevel 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Not in a git repository"
    }
    Set-Location $repoRoot
} catch {
    Write-Host "Error: Not in a git repository" -ForegroundColor Red
    exit 1
}

$remoteUrl = git remote get-url origin 2>$null
if ($remoteUrl) {
    Write-Host "Repository: $remoteUrl" -ForegroundColor Green
} else {
    Write-Host "Repository: No remote configured" -ForegroundColor Yellow
}
Write-Host ""

# Function to create issue
function Create-Issue {
    param(
        [int]$Num,
        [string]$Title,
        [string]$Labels,
        [string]$File
    )

    Write-Host "Creating Issue #${Num}: $Title" -ForegroundColor White

    try {
        $result = gh issue create `
            --title $Title `
            --label $Labels `
            --body-file $File 2>&1

        if ($LASTEXITCODE -eq 0) {
            Write-Host "✓ Created: $result" -ForegroundColor Green
        } else {
            Write-Host "✗ Failed: $result" -ForegroundColor Red
        }
        Write-Host ""
        Start-Sleep -Seconds 1  # Rate limiting
    } catch {
        Write-Host "✗ Failed to create issue #$Num" -ForegroundColor Red
        Write-Host $_.Exception.Message -ForegroundColor Red
        Write-Host ""
    }
}

# High Priority Issues (Sprint 5)
Write-Host "=== HIGH PRIORITY ISSUES (Sprint 5) ===" -ForegroundColor Yellow
Write-Host ""

Create-Issue -Num 1 `
    -Title "[HIGH] Rewrite nebula-parameter display system for nebula-validator compatibility" `
    -Labels "refactor,high-priority,nebula-parameter" `
    -File "docs/github_issues/issue_01_parameter_display_rewrite.md"

Create-Issue -Num 2 `
    -Title "[HIGH] Fix nebula-memory cache policy integration issues" `
    -Labels "bug,high-priority,nebula-memory" `
    -File "docs/github_issues/issue_02_memory_cache_policies.md"

Create-Issue -Num 3 `
    -Title "[HIGH] Implement pool maintenance and shutdown for nebula-resource" `
    -Labels "feature,high-priority,nebula-resource" `
    -File "docs/github_issues/issue_03_resource_pool_management.md"

Create-Issue -Num 4 `
    -Title "[HIGH] Implement ArenaGuard for RAII-based arena scope management" `
    -Labels "feature,high-priority,nebula-memory" `
    -File "docs/github_issues/issue_04_arena_scope_guard.md"

Create-Issue -Num 7 `
    -Title "[HIGH] Complete ResourceInstance test implementation with todo!() macros" `
    -Labels "bug,high-priority,nebula-resource,testing" `
    -File "docs/github_issues/issue_07_test_instance_todos.md"

# Medium Priority Issues (Sprint 6)
Write-Host "=== MEDIUM PRIORITY ISSUES (Sprint 6) ===" -ForegroundColor Yellow
Write-Host ""

Create-Issue -Num 5 `
    -Title "[MEDIUM] Re-enable and complete disabled modules across codebase" `
    -Labels "feature,medium-priority,technical-debt" `
    -File "docs/github_issues/issue_05_disabled_modules.md"

Create-Issue -Num 6 `
    -Title "[MEDIUM] Clean up excessive dead_code allows and unused functionality" `
    -Labels "refactor,medium-priority,code-quality" `
    -File "docs/github_issues/issue_06_dead_code_cleanup.md"

Create-Issue -Num 8 `
    -Title "[MEDIUM] Comprehensive unsafe code audit and documentation" `
    -Labels "security,medium-priority,nebula-memory,documentation" `
    -File "docs/github_issues/issue_08_unsafe_code_audit.md"

# Medium Priority Issues (Sprint 7)
Write-Host "=== MEDIUM PRIORITY ISSUES (Sprint 7) ===" -ForegroundColor Yellow
Write-Host ""

Create-Issue -Num 9 `
    -Title "[MEDIUM] Improve test coverage across all crates" `
    -Labels "testing,medium-priority,quality" `
    -File "docs/github_issues/issue_09_test_coverage.md"

Write-Host ""
Write-Host "=== SUMMARY ===" -ForegroundColor Cyan
Write-Host "✓ Script completed!" -ForegroundColor Green
Write-Host ""
Write-Host "Next steps:" -ForegroundColor White
Write-Host "1. Review created issues on GitHub" -ForegroundColor White
Write-Host "2. Assign issues to team members" -ForegroundColor White
Write-Host "3. Set up project board for tracking" -ForegroundColor White
Write-Host "4. Update TODO comments in code with issue numbers" -ForegroundColor White
Write-Host "5. Begin Sprint 5 development" -ForegroundColor White
Write-Host ""
Write-Host "View issues: gh issue list" -ForegroundColor Cyan
