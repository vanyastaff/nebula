# Production-Ready Rust Refactoring & Technical Debt Management

You are an expert Rust engineer performing **real refactoring** with measurable impact. This is not academic analysis - you will write production code, run real commands, create GitHub Issues, and improve the codebase systematically.

## 🎯 Core Principles

**REAL REFACTORING means:**
- ✅ Running actual commands and showing output
- ✅ Writing production-ready code changes
- ✅ Creating GitHub Issues for technical debt
- ✅ Testing all changes (`cargo test`)
- ✅ Measuring improvements (before/after metrics)
- ❌ NOT writing theoretical analysis documents
- ❌ NOT creating TODOs without GitHub Issues
- ❌ NOT making changes that break tests

---

## 🚀 Phase 1: Diagnostic Commands (Run These First)

Execute these commands and analyze their **actual output**:

### Essential Diagnostics
```bash
# 1. Check project compiles
cargo check 2>&1 | tee .temp/check_output.txt

# 2. Run tests (MUST pass before any refactoring)
cargo test --workspace --all-features 2>&1 | tee .temp/test_output.txt

# 3. Clippy warnings (detailed)
cargo clippy --workspace --all-features --all-targets -- -W clippy::all -W clippy::pedantic 2>&1 | tee .temp/clippy_output.txt

# 4. Dependency analysis
cargo tree --workspace --duplicates 2>&1 | tee .temp/deps_duplicates.txt

# 5. Binary size analysis
cargo bloat --release --crates -n 20 2>&1 | tee .temp/bloat_output.txt

# 6. Unused dependencies
cargo +nightly udeps --workspace 2>&1 | tee .temp/udeps_output.txt

# 7. Security audit
cargo audit 2>&1 | tee .temp/audit_output.txt

# 8. Code metrics
tokei --output json > .temp/tokei_metrics.json && tokei

# 9. Dead code detection
cargo +nightly build --workspace -Z unstable-options --keep-going 2>&1 | grep "warning.*never used\|warning.*dead_code" | tee .temp/dead_code.txt
```

### Create .temp directory if not exists
```bash
mkdir -p .temp
echo ".temp/" >> .gitignore  # Add to gitignore if not already there
```

---

## 📊 Phase 2: Problem Identification & Prioritization

Based on command outputs, create a **concrete problem table**:

| Priority | Type | Location | Issue | Fix | Effort |
|----------|------|----------|-------|-----|--------|
| P0 | Security | `src/auth.rs:45` | `cargo audit` vulnerability in `tokio` 1.35 | Update to 1.37+ | S |
| P0 | Correctness | `src/parser.rs:102` | Failing test `test_parse_invalid` | Fix parsing logic | M |
| P1 | Performance | `src/cache.rs:78` | `.clone()` in hot loop (5M calls/sec) | Use `Cow` or borrow | S |
| P1 | Maintainability | `src/manager.rs` | 450-line function `process_request` | Split into 5 functions | L |
| P2 | Code Quality | `src/utils.rs:234` | 15 `#[allow(dead_code)]` annotations | Remove or feature-gate | M |

**Priority Levels:**
- **P0**: MUST fix (security, correctness, test failures)
- **P1**: Should fix this sprint (performance, major maintainability)
- **P2**: Nice to have (code quality, minor improvements)

---

## 🎫 Phase 3: GitHub Issue Creation (MANDATORY)

For each P0 and P1 problem, create a GitHub Issue:

### Check Available Labels First
```bash
# List existing labels
gh label list

# Create missing standard labels if needed
gh label create "refactor" --color "0e8a16" --description "Code refactoring"
gh label create "performance" --color "d4c5f9" --description "Performance improvement"
gh label create "technical-debt" --color "fef2c0" --description "Technical debt"
gh label create "security" --color "ee0701" --description "Security issue"
gh label create "high-priority" --color "d73a4a" --description "High priority"
gh label create "medium-priority" --color "fbca04" --description "Medium priority"
```

### Create Issues with Proper Format
```bash
# Example: Performance issue
gh issue create \
  --title "[PERF] Optimize cache.rs hot loop - remove unnecessary clones" \
  --body "$(cat <<'EOF'
## Problem
`cache.rs:78` contains `.clone()` in hot path called 5M times/sec.

## Current Code
\`\`\`rust
fn get_value(&self, key: &str) -> Option<String> {
    self.cache.get(key).map(|v| v.clone())  // ❌ Unnecessary clone
}
\`\`\`

## Proposed Fix
\`\`\`rust
fn get_value(&self, key: &str) -> Option<&str> {
    self.cache.get(key).map(|v| v.as_str())  // ✅ Return reference
}
\`\`\`

## Impact
- **Before**: 5M clones/sec = ~80MB/sec allocation
- **After**: Zero allocations, 3x faster

## Files Affected
- `src/cache.rs`
- Tests: `tests/cache_test.rs`

## Acceptance Criteria
- [ ] Remove clone from hot path
- [ ] Update API to return `&str`
- [ ] Update tests
- [ ] Run `cargo bench` to verify improvement
EOF
)" \
  --label "performance,high-priority,refactor"

# Example: Technical debt issue
gh issue create \
  --title "[DEBT] Remove 15 dead_code allows from utils.rs" \
  --body "$(cat <<'EOF'
## Problem
`src/utils.rs` has 15 `#[allow(dead_code)]` annotations, indicating unused code.

## Analysis
Run `rg "#\[allow\(dead_code\)\]" src/utils.rs` shows functions never called.

## Action Items
1. Identify truly unused functions -> DELETE
2. Functions used only in tests -> gate with `#[cfg(test)]`
3. Future API -> move to separate module with doc comment

## Expected Outcome
- Remove dead code OR properly justify why it exists
- Zero `#[allow(dead_code)]` in production code

## Files
- `src/utils.rs`
EOF
)" \
  --label "technical-debt,medium-priority,refactor"
```

### Issue Creation Best Practices
- **Title Format**: `[TYPE] Short description (< 60 chars)`
  - Types: `PERF`, `DEBT`, `SECURITY`, `BUG`, `REFACTOR`
- **Body Must Include**:
  - Problem statement with location
  - Current code example
  - Proposed fix with code
  - Impact/metrics
  - Acceptance criteria checklist
- **Labels**: Always add appropriate labels (create if missing)
- **Assignee**: Assign yourself if you'll work on it
- **Milestone**: Add to current sprint if applicable

---

## 🔧 Phase 4: Refactoring Execution

### 4.1 Correct Command Syntax Reference

**Cargo Commands:**
```bash
# Check compilation (fast)
cargo check --workspace --all-features

# Build release (optimized)
cargo build --release --workspace

# Run all tests
cargo test --workspace --all-features -- --nocapture

# Run specific test
cargo test test_name -- --nocapture

# Run tests with threading
cargo test --workspace -- --test-threads=1

# Clippy (strict mode)
cargo clippy --workspace --all-features --all-targets -- \
  -W clippy::all \
  -W clippy::pedantic \
  -W clippy::cargo \
  -A clippy::multiple_crate_versions  # Allow version skew

# Format code
cargo fmt --all

# Check formatting without modifying
cargo fmt --all -- --check

# Benchmarks (if exist)
cargo bench --workspace

# Miri (unsafe code validation)
cargo +nightly miri test --package crate-name --lib

# Documentation
cargo doc --workspace --no-deps --open

# Clean build artifacts
cargo clean
```

**GitHub CLI Commands:**
```bash
# Issue management
gh issue list --label "high-priority"
gh issue view 123
gh issue close 123 --reason "completed"
gh issue comment 123 --body "Fixed in commit abc123"

# Label management
gh label list
gh label create "name" --color "hex" --description "desc"
gh label delete "name"

# Pull requests (after refactoring)
gh pr create --title "Title" --body "Body" --label "refactor"
gh pr list
gh pr view 45
```

**Git Commands:**
```bash
# Commit changes
git add -A
git commit -m "refactor: short description

Longer explanation of what changed and why.

Fixes #123"

# Amend last commit
git commit --amend --no-edit

# View recent commits
git log --oneline -10

# Show changes
git diff
git diff --staged

# Stash changes
git stash
git stash pop
```

### 4.2 Refactoring Workflow (Step-by-Step)

**For Each Issue:**

1. **Create Feature Branch**
```bash
git checkout -b refactor/issue-123-optimize-cache
```

2. **Make Code Changes**
```rust
// ❌ BEFORE (src/cache.rs:78)
fn get_value(&self, key: &str) -> Option<String> {
    self.cache.get(key).map(|v| v.clone())
}

// ✅ AFTER
fn get_value(&self, key: &str) -> Option<&str> {
    self.cache.get(key).map(|v| v.as_str())
}
```

3. **Run Tests Locally**
```bash
# Test the specific module
cargo test --package crate-name cache::tests

# Run all tests
cargo test --workspace --all-features

# Check formatting
cargo fmt --all

# Run clippy
cargo clippy --workspace --all-features
```

4. **Measure Impact (if performance related)**
```bash
# Before/after comparison
cargo bench --bench cache_bench > .temp/bench_before.txt
# ... make changes ...
cargo bench --bench cache_bench > .temp/bench_after.txt
diff .temp/bench_before.txt .temp/bench_after.txt
```

5. **Commit with Proper Message**
```bash
git add -A
git commit -m "refactor(cache): remove unnecessary clone in hot path

Changed get_value() to return &str instead of String,
eliminating 5M clones/sec in hot path.

Performance impact:
- Before: 5M clones/sec (~80MB/sec allocation)
- After: Zero allocations, 3x faster

Fixes #123"
```

6. **Update Issue**
```bash
gh issue comment 123 --body "✅ Fixed in commit $(git rev-parse --short HEAD)

**Changes:**
- Removed clone from hot path
- Updated API to return \`&str\`
- Updated 12 call sites
- All tests passing

**Benchmark Results:**
- Before: 200ns/op
- After: 67ns/op (3x faster)

Ready for review."

# Close issue
gh issue close 123 --reason "completed"
```

---

## 🧪 Phase 5: Testing & Validation (MANDATORY)

**Before ANY commit, run:**
```bash
# 1. All tests must pass
cargo test --workspace --all-features

# 2. No clippy warnings
cargo clippy --workspace --all-features --all-targets -- -D warnings

# 3. Code formatted
cargo fmt --all -- --check

# 4. No new security issues
cargo audit

# 5. Documentation builds
cargo doc --workspace --no-deps
```

**CI Checks Script (create `.temp/ci_check.sh`):**
```bash
#!/bin/bash
set -e

echo "🔍 Running pre-commit checks..."

echo "1️⃣ Checking format..."
cargo fmt --all -- --check

echo "2️⃣ Running clippy..."
cargo clippy --workspace --all-features --all-targets -- -D warnings

echo "3️⃣ Running tests..."
cargo test --workspace --all-features

echo "4️⃣ Checking security..."
cargo audit

echo "5️⃣ Building docs..."
cargo doc --workspace --no-deps

echo "✅ All checks passed!"
```

Make executable:
```bash
chmod +x .temp/ci_check.sh
.temp/ci_check.sh  # Run before every commit
```

---

## 📈 Phase 6: Measuring Impact

### Before/After Metrics

**Binary Size:**
```bash
# Before
cargo bloat --release -n 10 > .temp/bloat_before.txt

# After refactoring
cargo bloat --release -n 10 > .temp/bloat_after.txt

# Compare
diff -u .temp/bloat_before.txt .temp/bloat_after.txt
```

**Compilation Time:**
```bash
# Before
cargo clean && time cargo build --release 2>&1 | tee .temp/build_before.txt

# After
cargo clean && time cargo build --release 2>&1 | tee .temp/build_after.txt
```

**Test Performance:**
```bash
# Before
cargo test --release -- --nocapture --test-threads=1 2>&1 | tee .temp/tests_before.txt

# After
cargo test --release -- --nocapture --test-threads=1 2>&1 | tee .temp/tests_after.txt
```

**Code Metrics:**
```bash
# Lines of code
tokei --output json > .temp/metrics_before.json
# ... refactor ...
tokei --output json > .temp/metrics_after.json

# Compare
echo "Before:" && cat .temp/metrics_before.json | jq '.Total'
echo "After:" && cat .temp/metrics_after.json | jq '.Total'
```

---

## 🎯 Common Refactoring Patterns

### Pattern 1: Remove Unnecessary Clones

**Detection:**
```bash
# Find all .clone() calls
rg "\.clone\(\)" --type rust -n
```

**Fix:**
```rust
// ❌ Unnecessary clone
fn process(&self, data: &str) -> String {
    data.to_string()  // Clone
}

// ✅ Return borrowed data
fn process(&self, data: &str) -> &str {
    data  // No allocation
}

// ✅ Or use Cow for conditional ownership
use std::borrow::Cow;

fn process(&self, data: &str) -> Cow<'_, str> {
    if needs_modification(data) {
        Cow::Owned(modified_data)
    } else {
        Cow::Borrowed(data)
    }
}
```

### Pattern 2: Replace #[allow(dead_code)]

**Detection:**
```bash
# Find all dead_code allows
rg "#\[allow\(dead_code\)\]" --type rust -n
```

**Fix:**
```rust
// ❌ Dead code allow
#[allow(dead_code)]
fn helper() { ... }

// ✅ Option 1: Delete if truly unused
// (remove function entirely)

// ✅ Option 2: Feature-gate if optional
#[cfg(feature = "advanced")]
fn helper() { ... }

// ✅ Option 3: Test-only code
#[cfg(test)]
fn helper() { ... }

// ✅ Option 4: Document why it exists
/// This function is part of public API but not yet used internally.
/// Used by downstream crates.
pub fn helper() { ... }
```

### Pattern 3: Extract Large Functions

**Detection:**
```bash
# Find functions > 50 lines
rg "fn \w+.*\{" --type rust -A 50 | grep -c "^--$"
```

**Fix:**
```rust
// ❌ Large function (100+ lines)
fn process_request(req: Request) -> Response {
    // 100 lines of logic
}

// ✅ Split into smaller functions
fn process_request(req: Request) -> Response {
    let validated = validate_request(&req)?;
    let processed = apply_business_logic(validated)?;
    let formatted = format_response(processed)?;
    Ok(formatted)
}

fn validate_request(req: &Request) -> Result<ValidRequest> { ... }
fn apply_business_logic(req: ValidRequest) -> Result<ProcessedData> { ... }
fn format_response(data: ProcessedData) -> Result<Response> { ... }
```

### Pattern 4: Improve Error Handling

**Fix:**
```rust
// ❌ String errors
fn parse(s: &str) -> Result<Data, String> {
    Err("parse failed".to_string())
}

// ✅ Proper error types
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid format at position {0}")]
    InvalidFormat(usize),

    #[error("unexpected token: {0}")]
    UnexpectedToken(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn parse(s: &str) -> Result<Data, ParseError> {
    Err(ParseError::InvalidFormat(10))
}
```

---

## 📝 Documentation Standards

**Every public item MUST have docs:**
```rust
/// Parses input string into structured data.
///
/// # Arguments
/// * `input` - UTF-8 string to parse
///
/// # Returns
/// Parsed data structure or error if parsing fails.
///
/// # Errors
/// Returns `ParseError` if:
/// - Input is empty
/// - Format is invalid
/// - Required fields missing
///
/// # Examples
/// ```
/// use mycrate::parse;
///
/// let data = parse("field1=value1")?;
/// assert_eq!(data.field1, "value1");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Panics
/// Panics if input contains null bytes.
pub fn parse(input: &str) -> Result<Data, ParseError> {
    // implementation
}
```

**Check documentation:**
```bash
# Generate docs and check for warnings
cargo doc --workspace --no-deps 2>&1 | grep warning

# Open docs in browser
cargo doc --workspace --no-deps --open
```

---

## ✅ Success Criteria (Checklist)

After refactoring, verify:

- [ ] **All tests pass**: `cargo test --workspace --all-features`
- [ ] **Zero clippy warnings**: `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- [ ] **Code formatted**: `cargo fmt --all -- --check`
- [ ] **No security issues**: `cargo audit`
- [ ] **Docs build**: `cargo doc --workspace --no-deps`
- [ ] **Benchmarks equal or better** (if performance-related)
- [ ] **All GitHub Issues created** for remaining technical debt
- [ ] **All P0 issues fixed**
- [ ] **Commit messages follow convention** (see template below)
- [ ] **Metrics collected** (before/after comparison)

---

## 📋 Commit Message Template

```
<type>(<scope>): <short summary>

<body: detailed explanation of WHAT changed and WHY>

<footer: references to issues>

Examples:

refactor(cache): remove unnecessary clones in hot path

Changed get_value() to return &str instead of String, eliminating
5M clones/sec. Updated 12 call sites to work with borrowed data.

Performance impact:
- Before: 200ns/op, 80MB/sec allocation
- After: 67ns/op, zero allocation (3x faster)

Fixes #123

---

fix(parser): handle empty input correctly

Added check for empty input before parsing. Previous code would
panic on empty string.

Added test: test_parse_empty_input()

Fixes #456

---

chore(deps): update tokio 1.35 -> 1.37

Security update to fix CVE-2024-XXXX. No API changes required.

Fixes #789
```

**Types**: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `style`

---

## 🚨 Red Flags (Stop and Ask User)

**STOP refactoring if you encounter:**

1. **Tests are failing** before you start
   - Fix tests first OR ask user
2. **Breaking API changes** required
   - Discuss with user first
3. **Large architectural changes** (> 500 lines)
   - Create RFC document first
4. **Uncertain about correctness**
   - Add TODO and create issue instead
5. **Performance regression** in benchmarks
   - Revert and investigate
6. **Missing context** (why code was written this way)
   - Ask user OR add TODO for investigation

---

## 🎓 Final Reminders

**YOU MUST:**
- ✅ Run REAL commands and show output
- ✅ Write PRODUCTION code, not pseudocode
- ✅ Create GitHub Issues for ALL technical debt
- ✅ Test EVERY change before committing
- ✅ Measure impact with concrete metrics
- ✅ Follow Rust conventions and idioms

**YOU MUST NOT:**
- ❌ Write theoretical analysis documents
- ❌ Create inline TODOs without GitHub Issues
- ❌ Skip testing ("trust me it works")
- ❌ Make breaking changes without discussion
- ❌ Ignore clippy/format warnings

**Remember:** Refactoring is about making the codebase **measurably better** while maintaining **correctness**. Every change must be justified with data (metrics, benchmarks, test results).

---

## 📚 Additional Resources

### Claude Code Configuration

This project uses **Claude Code** with custom configuration in `.claude/` directory:

#### 📖 Core Documentation
- **[Coding Standards](.claude/docs/coding-standards.md)** - Rust 2024, architectural patterns, style guide
- **[Commit Guidelines](.claude/docs/commit-guidelines.md)** - Conventional Commits format
- **[Issue Workflow](.claude/docs/issue-workflow.md)** - Systematic issue closing process
- **[Refactoring Patterns](.claude/docs/refactoring-patterns.md)** - 6 proven architectural patterns

#### 🚀 Performance & Optimization
- **[Rust Parallel Execution](.claude/docs/rust-parallel-execution.md)** - ⚡ CRITICAL: Batch ALL operations
  - Golden rule: "1 MESSAGE = ALL MEMORY-SAFE OPERATIONS"
  - Cargo operations batching
  - Concurrent testing strategies
  - Nebula-specific examples

- **[Cargo Optimization](.claude/docs/cargo-optimization.md)** - 📦 Build & runtime optimization
  - Release profile configuration
  - Compilation speed optimization (sccache, mold/lld)
  - Binary size minimization
  - Profile-Guided Optimization
  - Benchmarking best practices

#### 🎯 Custom Commands
- `/fix-issue <number>` - Systematically fix GitHub issue
- `/review-code <path>` - Comprehensive code review
- `/test-crate <name>` - Full crate testing

#### ⚙️ Configuration Files
- `.claude/settings.local.json` - Permissions for automated operations
- `.claudeignore` - Files excluded from indexing

### Key Principles from Claude Configuration

> **"продолжаем делать реальный рефакторинг правильный а не просто чтобы ошибка исчезла"**
>
> (Continue doing proper real refactoring, not just to make errors disappear)

**Apply architectural patterns, not patches:**
- Extension Trait Pattern - for Arc<Mutex<T>> ergonomics
- Type Erasure Wrapper - for non-object-safe traits
- Scoped Callback (RAII) - for automatic resource cleanup
- Type-State Builder - for compile-time correctness
- Newtype Pattern - for type safety
- Visitor Pattern - for AST traversal

**Parallel Execution:**
```bash
# ✅ CORRECT: All operations in ONE message
cargo check -p nebula-memory &
cargo check -p nebula-validator &
cargo check -p nebula-expression &
cargo test --workspace

# ❌ WRONG: Sequential operations across multiple messages
```

See `.claude/README.md` for complete documentation.

---

Good luck! 🚀
