# GitHub Issues for Technical Debt

This directory contains GitHub issue templates for high-priority technical debt items identified in the Nebula project.

## How to Use

1. Review each issue template
2. Create GitHub issues using these templates
3. Link issues in the [Technical Debt Tracker](../TECHNICAL_DEBT.md)
4. Update issue numbers in TODO comments when created

## High Priority Issues

### Sprint 5 Target

| # | Issue | Priority | Labels | Description |
|---|-------|----------|--------|-------------|
| 1 | [Parameter Display System Rewrite](issue_01_parameter_display_rewrite.md) | 🔴 HIGH | refactor, nebula-parameter | Rewrite display system for nebula-validator compatibility |
| 2 | [Memory Cache Policy Integration](issue_02_memory_cache_policies.md) | 🔴 HIGH | bug, nebula-memory | Fix cache policy type mismatches and LFU module |
| 3 | [Resource Pool Management](issue_03_resource_pool_management.md) | 🔴 HIGH | feature, nebula-resource | Implement pool maintenance and shutdown |
| 4 | [Arena Scope Guard](issue_04_arena_scope_guard.md) | 🔴 HIGH | feature, nebula-memory | Implement RAII-based arena scope management |

## Creating Issues on GitHub

### Using GitHub CLI

```bash
# Issue 1: Parameter Display System
gh issue create \
  --title "[HIGH] Rewrite nebula-parameter display system for nebula-validator compatibility" \
  --label "refactor,high-priority,nebula-parameter" \
  --body-file docs/github_issues/issue_01_parameter_display_rewrite.md

# Issue 2: Memory Cache Policies
gh issue create \
  --title "[HIGH] Fix nebula-memory cache policy integration issues" \
  --label "bug,high-priority,nebula-memory" \
  --body-file docs/github_issues/issue_02_memory_cache_policies.md

# Issue 3: Resource Pool Management
gh issue create \
  --title "[HIGH] Implement pool maintenance and shutdown for nebula-resource" \
  --label "feature,high-priority,nebula-resource" \
  --body-file docs/github_issues/issue_03_resource_pool_management.md

# Issue 4: Arena Scope Guard
gh issue create \
  --title "[HIGH] Implement ArenaGuard for RAII-based arena scope management" \
  --label "feature,high-priority,nebula-memory" \
  --body-file docs/github_issues/issue_04_arena_scope_guard.md
```

### Using GitHub Web Interface

1. Go to repository Issues page
2. Click "New Issue"
3. Copy content from corresponding markdown file
4. Set title, labels, and milestone as specified
5. Submit issue

## After Creating Issues

1. Note the issue numbers
2. Update TODO comments in code with issue references:
   ```rust
   // TODO(#123): Implement display system
   ```
3. Update [TECHNICAL_DEBT.md](../TECHNICAL_DEBT.md) with issue links
4. Link issues in project board if using GitHub Projects

## Labels Used

- `high-priority` - Critical items for Sprint 5
- `refactor` - Code restructuring
- `bug` - Bug fixes required
- `feature` - New functionality
- Crate-specific: `nebula-parameter`, `nebula-memory`, `nebula-resource`, etc.

## Milestones

- **Sprint 5**: All high-priority technical debt items
- **Sprint 6-7**: Medium priority items (to be added)
- **Backlog**: Low priority items

## Related Documents

- [Technical Debt Tracker](../TECHNICAL_DEBT.md) - Complete technical debt inventory
- [Rust Refactoring Guide](../rust_refactor_prompt.md) - Development guidelines
- [Contributing Guidelines](../../CONTRIBUTING.md) - How to contribute

## Tracking Progress

Update issue status regularly:
- **Open**: Not started
- **In Progress**: Active development
- **In Review**: Pull request submitted
- **Closed**: Completed and merged

## Template Format

Each issue template includes:
- **Problem**: What needs to be fixed/implemented
- **Current State**: Code locations and status
- **Impact**: Why this is important
- **Action Items**: Specific tasks to complete
- **Files Affected**: Which files need changes
- **Technical Details**: Implementation guidance
- **References**: Links to related documentation
- **Acceptance Criteria**: Definition of done

## Next Steps

1. ✅ Issues created (see templates in this directory)
2. ⏳ Create issues on GitHub (manual or via CLI)
3. ⏳ Update TECHNICAL_DEBT.md with issue links
4. ⏳ Begin Sprint 5 development

---

*Generated: 2025-10-10*
*Sprint: 4 (Technical Debt Documentation)*
