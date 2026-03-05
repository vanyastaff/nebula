# Development Workflow

This document outlines how we manage branches, commits, pull requests, and releases in Nebula.

---

## Table of Contents

- [Branch Strategy](#branch-strategy)
- [Branch Naming](#branch-naming)
- [Commit Conventions](#commit-conventions)
- [Pull Request Process](#pull-request-process)
- [Code Review Guidelines](#code-review-guidelines)
- [Testing Requirements](#testing-requirements)
- [Versioning & Releases](#versioning--releases)

---

## Branch Strategy

We use a simplified **trunk-based development** approach:

```
main (stable, always deployable)
  ↑
  └─ feature/xyz (development)
  └─ fix/abc (bug fixes)
  └─ docs/readme (documentation)
```

### Main Branch (`main`)

- **Protected branch** — All changes via pull requests
- **Always deployable** — Tests pass, code reviewed
- **Release source** — Releases are tagged from `main`

### Feature/Fix Branches

- Created from `main`
- Deleted after merge
- Short-lived (aim for < 2 weeks)

### Long-Lived Branches?

**No.** If you find yourself needing a long-lived branch (e.g., `develop`, `staging`), it's a sign to:
- Break work into smaller PRs
- Coordinate with maintainers
- Document the exception in a GitHub issue

---

## Branch Naming

Use descriptive, lowercase names with forward slashes:

```
{category}/{short-description}
```

### Categories

| Category | Use Case | Example |
|----------|----------|---------|
| `feat/` | New feature | `feat/action-retries` |
| `fix/` | Bug fix | `fix/credential-injection-panic` |
| `docs/` | Documentation | `docs/architecture-guide` |
| `refactor/` | Code cleanup (no behavior change) | `refactor/executor-simplify` |
| `test/` | Adding/improving tests | `test/coverage-engine` |
| `chore/` | Build, CI, dependencies | `chore/upgrade-tokio` |
| `perf/` | Performance improvement | `perf/reduce-allocations` |

### Rules

- ✅ Use hyphens to separate words: `feat/action-retry-backoff`
- ❌ Don't use underscores: `feat/action_retry`
- ❌ Don't use spaces: `feat/action retry`
- ✅ Include issue number if applicable: `fix/credential-injection-panic-456`
- ✅ Keep it short: `fix/panic` → OK, `fix/fix-the-panic-that-happens-when-you-delete-credentials` → Too long

---

## Commit Conventions

We follow **Conventional Commits**. Each commit message has this structure:

```
<type>(<scope>): <subject>

<body>

<footer>
```

### Type

Must be one of:

- `feat:` — New feature
- `fix:` — Bug fix
- `docs:` — Documentation
- `test:` — Test additions/updates
- `refactor:` — Code refactoring (no behavior change)
- `perf:` — Performance improvement
- `chore:` — Build, CI, dependencies

### Scope

Optional. Name of the affected crate/module:

```
feat(runtime): add action timeout
fix(credential): prevent panic on deletion
docs(architecture): clarify async patterns
```

### Subject

- Imperative mood: "add" not "added" or "adds"
- Don't capitalize: `fix` not `Fix`
- No period at end: `fix: bug` not `fix: bug.`
- Limit to 50 characters

### Body

Optional. Explain *what* and *why*, not *how*:

```
feat(engine): implement workflow versioning

This allows workflows to maintain multiple versions and enables
safe updates. When a new version is created, previous versions
remain executable for running instances.

This solves the problem of breaking changes in workflows that
are still in use by existing executions.
```

### Footer

Optional. Reference issues:

```
Closes #123
Related to #456
Refs: #789
```

### Examples

✅ **Good:**
```
feat(action): add conditional branching

Allows workflows to branch execution based on previous node output.
Supports both if/else and switch patterns.

Closes #42
```

```
fix(runtime): prevent panics in credential injection

Move unwrap() calls to proper error handling to prevent panics
when a credential is deleted mid-execution.

Related to #456
```

```
docs(crates): add contributing guide for action authors
```

❌ **Bad:**
```
fix: stuff
Updated code
Changed things
Better error handling
```

---

## Pull Request Process

### Before Creating a PR

1. **Sync with main**:
   ```bash
   git checkout main
   git pull upstream main
   git checkout your-branch
   git merge main
   ```

2. **Run tests locally**:
   ```bash
   cargo test
   ```

3. **Check code quality**:
   ```bash
   cargo clippy -- -D warnings
   cargo fmt --check
   ```

4. **Ensure commit messages follow conventions** (see above)

### Creating a PR

Use this template (auto-filled by GitHub):

```markdown
## Description
Brief explanation of what this PR does.

## Related Issue
Closes #123

## Type of Change
- [ ] New feature
- [ ] Bug fix
- [ ] Breaking change
- [ ] Documentation

## Testing
How did you test this? Include steps to reproduce.

## Checklist
- [ ] Tests pass: `cargo test`
- [ ] Clippy passes: `cargo clippy -- -D warnings`
- [ ] Code formatted: `cargo fmt`
- [ ] Commits follow conventions
- [ ] PR description is clear
- [ ] No breaking changes or discussed with maintainers
- [ ] Documentation updated (if needed)

## Screenshots (if applicable)
```

### Size Guidelines

- **Small PR** (< 200 lines): 1–2 days to review
- **Medium PR** (200–500 lines): 2–5 days
- **Large PR** (> 500 lines): Schedule discussion with maintainers first

**Too big?** Break it into smaller PRs. Example:
- PR 1: Refactor, no behavior change
- PR 2: Add tests
- PR 3: Implement feature (now smaller due to refactoring)

### Merging

- **Squash commits** if your PR has many small "fix typo" commits
- **Use descriptive merge commit message**: "feat(engine): implement workflow versioning (#42)"
- **Delete branch** after merge

---

## Code Review Guidelines

### What Reviewers Look For

#### ✅ Correctness
- Tests pass
- Logic is sound
- Error handling is complete
- No panics or unwraps without good reason

#### ✅ Design
- Follows project architecture
- One-way dependencies respected
- Trait contracts upheld
- No breaking changes without discussion

#### ✅ Performance
- No unnecessary allocations
- Async patterns are correct (no blocking calls in async)
- No unbounded queues

#### ✅ Readability
- Code is clear and self-documenting
- Comments explain *why*, not *what*
- Variable names are descriptive
- Clippy warnings addressed

#### ✅ Testing
- Happy path tested
- Edge cases covered
- Error cases tested
- No flaky tests

### As a Reviewer

- Be respectful and constructive
- Explain *why* something should change
- Offer alternatives if you're suggesting refactoring
- Approve when tests pass and design is sound

### As a PR Author

- Respond to feedback promptly
- Ask for clarification if unclear
- Suggest alternatives if you disagree
- Request re-review after updates

---

## Testing Requirements

### Unit Tests

Every public function should have tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_execution_success() {
        // Arrange
        let action = TestAction::new();
        
        // Act
        let result = action.execute().await;
        
        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_action_handles_timeout() {
        // Test timeout behavior
    }
}
```

### Integration Tests

Place in `crates/*/tests/`:

```bash
crates/engine/tests/dag_execution.rs
crates/runtime/tests/isolation.rs
```

### Running Tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p nebula-engine

# With output
cargo test -- --nocapture

# Only unit tests (no integration)
cargo test --lib

# Only integration tests
cargo test --test '*'
```

### Coverage

- Aim for > 70% coverage on public APIs
- Use `cargo tarpaulin` to measure

---

## Versioning & Releases

We follow **Semantic Versioning** (`MAJOR.MINOR.PATCH`):

- `0.y.z` — Pre-release (breaking changes may occur)
- `1.0.0+` — Stable (respect semver)

### Release Process

1. **Update version** in `Cargo.toml` (all crates):
   ```toml
   version = "0.2.0"
   ```

2. **Update CHANGELOG** with highlights and breaking changes

3. **Create release PR** with title: `chore: release 0.2.0`

4. **Merge to main** (requires approval)

5. **Tag release**:
   ```bash
   git tag -a v0.2.0 -m "Release 0.2.0"
   git push origin v0.2.0
   ```

6. **Create GitHub Release** from tag with release notes

7. **Publish to crates.io**:
   ```bash
   cargo publish
   ```

### Breaking Changes

During pre-1.0, breaking changes are allowed but:

- Must be documented in PR description
- Must be discussed in an issue first
- Must update migration guide in CHANGELOG

---

## Troubleshooting

### "I have merge conflicts"

```bash
git fetch upstream
git merge upstream/main
# Resolve conflicts in editor
git add .
git commit -m "Resolve merge conflicts"
git push
```

### "I committed with wrong message"

```bash
# Last commit only
git commit --amend

# Older commits
git rebase -i upstream/main
# Mark commits as 'reword'
```

### "I want to discard local changes"

```bash
# Single file
git checkout crates/engine/src/lib.rs

# All changes
git reset --hard upstream/main
```

### "How do I sync my fork?"

```bash
git remote add upstream https://github.com/vanyastaff/nebula.git
git fetch upstream
git checkout main
git merge upstream/main
git push origin main
```

---

**Questions?** See [CONTRIBUTING.md](CONTRIBUTING.md) or open a [discussion](https://github.com/vanyastaff/nebula/discussions).

