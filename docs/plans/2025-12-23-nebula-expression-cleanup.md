# nebula-expression Crate Cleanup & Optimization Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Review and clean up nebula-expression crate using TDD, fix compilation errors, optimize performance, improve documentation, apply advanced Rust patterns, and address security concerns.

**Architecture:** The crate implements an expression language with lexer → parser → AST → evaluator pipeline, featuring caching, template rendering, and 60+ built-in functions. Focus areas: fix test compilation, implement missing lambda support, add ReDoS protection, improve type safety.

**Tech Stack:** Rust 1.92, thiserror, serde, regex, chrono, parking_lot, once_cell, nebula-value, nebula-memory

---

## Phase 1: Fix Compilation Errors (Critical)

### Task 1: Fix Test Type Mismatches in engine.rs

**Files:**
- Modify: `crates/nebula-expression/src/engine.rs:259-508`

**Step 1: Read the current test file to understand Integer type usage**

The tests use `Some(42)` but `as_integer()` returns `Option<Integer>`, not `Option<i64>`.

**Step 2: Write the fix for test_evaluate_literal**

```rust
// In engine.rs tests, change:
assert_eq!(result.as_integer(), Some(42));
// To:
assert_eq!(result.as_integer(), Some(nebula_value::Integer::new(42)));
```

**Step 3: Apply fix to all affected tests in engine.rs**

Fix lines: 259, 268, 286, 413, 417, 432, 435, 481, 508

Replace each `Some(<number>)` with `Some(nebula_value::Integer::new(<number>))`.

**Step 4: Run tests to verify fixes**

Run: `cargo test -p nebula-expression --lib -- engine::tests`
Expected: All engine tests PASS

**Step 5: Commit**

```bash
git add crates/nebula-expression/src/engine.rs
git commit -m "fix(nebula-expression): use Integer::new() in engine tests"
```

---

### Task 2: Fix Test Type Mismatches in eval/mod.rs

**Files:**
- Modify: `crates/nebula-expression/src/eval/mod.rs:554-589`

**Step 1: Fix test_eval_literal**

```rust
// Line 554: Change
assert_eq!(result.as_integer(), Some(42));
// To:
assert_eq!(result.as_integer(), Some(nebula_value::Integer::new(42)));
```

**Step 2: Fix test_eval_arithmetic**

```rust
// Line 567: Change
assert_eq!(result.as_integer(), Some(15));
// To:
assert_eq!(result.as_integer(), Some(nebula_value::Integer::new(15)));
```

**Step 3: Fix test_deep_nesting_within_limit**

```rust
// Line 589: Change
assert_eq!(result.unwrap().as_integer(), Some(51));
// To:
assert_eq!(result.unwrap().as_integer(), Some(nebula_value::Integer::new(51)));
```

**Step 4: Run tests to verify fixes**

Run: `cargo test -p nebula-expression --lib -- eval::tests`
Expected: All eval tests PASS

**Step 5: Commit**

```bash
git add crates/nebula-expression/src/eval/mod.rs
git commit -m "fix(nebula-expression): use Integer::new() in eval tests"
```

---

### Task 3: Fix Invalid Feature Flag in engine.rs

**Files:**
- Modify: `crates/nebula-expression/src/engine.rs:419,467,488`

**Step 1: Identify the issue**

The code uses `#[cfg(feature = "std")]` but the valid features are: `cache`, `datetime`, `default`, `full`, `regex`, `uuid`. The `std` feature doesn't exist.

**Step 2: Remove or fix the invalid cfg attributes**

```rust
// Remove these blocks entirely (lines 419, 467, 488) since they reference non-existent feature
// The code inside these blocks checks cache stats which always return None anyway
```

**Step 3: Run check to verify**

Run: `cargo check -p nebula-expression --all-features`
Expected: No warnings about unexpected cfg conditions

**Step 4: Commit**

```bash
git add crates/nebula-expression/src/engine.rs
git commit -m "fix(nebula-expression): remove invalid std feature cfg"
```

---

### Task 4: Fix Integration Test Type Mismatches

**Files:**
- Modify: `crates/nebula-expression/tests/integration_test.rs`

**Step 1: Add import for Integer type**

```rust
use nebula_value::{Value, Integer};
```

**Step 2: Fix all as_integer() comparisons**

Replace pattern `Some(<number>)` with `Some(Integer::new(<number>))` on lines using `as_integer()`.

**Step 3: Run integration tests**

Run: `cargo test -p nebula-expression --test integration_test`
Expected: All integration tests PASS

**Step 4: Commit**

```bash
git add crates/nebula-expression/tests/integration_test.rs
git commit -m "fix(nebula-expression): use Integer::new() in integration tests"
```

---

### Task 5: Verify All Tests Pass

**Step 1: Run full test suite**

Run: `cargo test -p nebula-expression --all-features`
Expected: All tests PASS

**Step 2: Run clippy**

Run: `cargo clippy -p nebula-expression --all-features -- -D warnings`
Expected: No errors (warnings from dependencies OK)

**Step 3: Commit if any additional fixes needed**

---

## Phase 2: Code Quality & Cleanup

### Task 6: Remove Dead Code - extract_lambda Function

**Files:**
- Modify: `crates/nebula-expression/src/builtins/mod.rs`

**Step 1: Write test proving extract_lambda is used or remove it**

The function has `#[allow(dead_code)]` - check if it's actually needed.

```rust
#[test]
fn test_extract_lambda_exists_for_future_use() {
    // If filter/map/reduce are implemented, this helper will be needed
    // For now, mark with proper documentation
}
```

**Step 2: Either implement lambda support OR document why it's kept**

Add documentation:
```rust
/// Helper to extract a lambda expression from args
///
/// Note: Currently unused as filter/map/reduce are stubs.
/// Will be activated when lambda evaluation is implemented.
#[allow(dead_code)]
pub(crate) fn extract_lambda(arg: &Expr) -> ExpressionResult<(&str, &Expr)> {
```

**Step 3: Commit**

```bash
git add crates/nebula-expression/src/builtins/mod.rs
git commit -m "docs(nebula-expression): document extract_lambda future use"
```

---

### Task 7: Add Missing Documentation to Public Items

**Files:**
- Modify: `crates/nebula-expression/src/lib.rs`
- Modify: `crates/nebula-expression/src/context/mod.rs`

**Step 1: Add module-level docs to context**

Read context/mod.rs and add comprehensive documentation.

**Step 2: Ensure all public types have docs**

Check each public export has `///` documentation.

**Step 3: Run doc check**

Run: `cargo doc -p nebula-expression --no-deps`
Expected: No documentation warnings

**Step 4: Commit**

```bash
git add crates/nebula-expression/src/
git commit -m "docs(nebula-expression): add missing documentation"
```

---

## Phase 3: Security Improvements

### Task 8: Add ReDoS Protection to Regex Matching

**Files:**
- Modify: `crates/nebula-expression/src/eval/mod.rs`
- Modify: `crates/nebula-expression/Cargo.toml`

**Step 1: Write failing test for ReDoS protection**

```rust
#[test]
#[cfg(feature = "regex")]
fn test_regex_timeout_protection() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    
    // This pattern could cause catastrophic backtracking
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::text("aaaaaaaaaaaaaaaaaaaaaaaa!"))),
        op: BinaryOp::RegexMatch,
        right: Box::new(Expr::Literal(Value::text("(a+)+$"))),
    };
    
    // Should complete in reasonable time (not hang)
    let start = std::time::Instant::now();
    let _ = evaluator.eval(&expr, &context);
    assert!(start.elapsed() < std::time::Duration::from_secs(1), 
            "Regex should timeout or complete quickly, not hang");
}
```

**Step 2: Run test to verify it fails (or hangs)**

Run: `cargo test -p nebula-expression --lib -- test_regex_timeout_protection --timeout 5`
Expected: Test may hang or take too long

**Step 3: Implement regex size/complexity limits**

```rust
/// Maximum length for regex patterns to prevent ReDoS
const MAX_REGEX_PATTERN_LEN: usize = 1000;

/// Check if pattern contains potentially dangerous constructs
fn is_safe_regex_pattern(pattern: &str) -> bool {
    if pattern.len() > MAX_REGEX_PATTERN_LEN {
        return false;
    }
    // Reject patterns with nested quantifiers like (a+)+
    // Simple heuristic: count nested groups with quantifiers
    let mut depth = 0;
    let mut has_quantifier_in_group = false;
    for c in pattern.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '+' | '*' | '?' if depth > 0 => has_quantifier_in_group = true,
            _ => {}
        }
    }
    // If we have quantifiers inside groups AND the pattern ends with +/*
    !(has_quantifier_in_group && pattern.ends_with(|c| c == '+' || c == '*'))
}
```

**Step 4: Integrate check into regex_match function**

```rust
#[cfg(feature = "regex")]
fn regex_match(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
    // ... existing code ...
    
    // Add safety check before compilation
    if !is_safe_regex_pattern(pattern) {
        return Err(ExpressionError::expression_regex_error(
            "Regex pattern rejected: too complex or potentially unsafe"
        ));
    }
    
    // ... rest of function ...
}
```

**Step 5: Run test to verify it passes**

Run: `cargo test -p nebula-expression --lib -- test_regex_timeout_protection`
Expected: PASS (pattern rejected as unsafe)

**Step 6: Commit**

```bash
git add crates/nebula-expression/src/eval/mod.rs
git commit -m "security(nebula-expression): add ReDoS protection for regex"
```

---

### Task 9: Add Regex Cache Size Limit

**Files:**
- Modify: `crates/nebula-expression/src/eval/mod.rs`

**Step 1: Write test for cache size limit**

```rust
#[test]
#[cfg(feature = "regex")]
fn test_regex_cache_size_limit() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    
    // Insert many different patterns
    for i in 0..200 {
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::text("test"))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::text(&format!("pattern{}", i)))),
        };
        let _ = evaluator.eval(&expr, &context);
    }
    
    // Cache should not exceed limit
    assert!(evaluator.regex_cache.lock().len() <= MAX_REGEX_CACHE_SIZE);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-expression --lib -- test_regex_cache_size_limit`
Expected: FAIL (cache grows unbounded)

**Step 3: Add LRU-style eviction to regex cache**

```rust
/// Maximum number of cached regex patterns
const MAX_REGEX_CACHE_SIZE: usize = 100;

// In regex_match, after inserting new pattern:
if cache.len() > MAX_REGEX_CACHE_SIZE {
    // Simple eviction: remove a random entry
    if let Some(key) = cache.keys().next().cloned() {
        cache.remove(&key);
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-expression --lib -- test_regex_cache_size_limit`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/nebula-expression/src/eval/mod.rs
git commit -m "perf(nebula-expression): add regex cache size limit"
```

---

## Phase 4: Advanced Rust Patterns & Optimization

### Task 10: Use Cow for String Operations

**Files:**
- Modify: `crates/nebula-expression/src/builtins/string.rs`

**Step 1: Write benchmark test for string operations**

```rust
#[test]
fn test_trim_no_alloc_when_unchanged() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();
    
    // String with no whitespace should not allocate
    let result = engine.evaluate("trim('hello')", &context).unwrap();
    assert_eq!(result.as_str(), Some("hello"));
}
```

**Step 2: Optimize trim to use Cow**

```rust
use std::borrow::Cow;

pub fn trim(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("trim", args, 1)?;
    let s = get_string_arg("trim", args, 0, "string")?;
    let trimmed = s.trim();
    
    // Avoid allocation if string unchanged
    if trimmed.len() == s.len() {
        Ok(args[0].clone())  // Return original Value
    } else {
        Ok(Value::text(trimmed))
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p nebula-expression --lib -- builtins::string`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/nebula-expression/src/builtins/string.rs
git commit -m "perf(nebula-expression): optimize trim to avoid allocation"
```

---

### Task 11: Add #[inline] Hints for Hot Path Functions

**Files:**
- Modify: `crates/nebula-expression/src/eval/mod.rs`
- Modify: `crates/nebula-expression/src/lexer/mod.rs`

**Step 1: Identify hot path functions**

- `Evaluator::eval_with_depth` - called for every AST node
- `Lexer::current_char`, `advance`, `peek` - called for every character

**Step 2: Add inline hints**

```rust
// In eval/mod.rs
#[inline]
fn eval_with_depth(...) -> ExpressionResult<Value> {

// In lexer/mod.rs
#[inline]
fn current_char(&self) -> Option<char> {

#[inline]
fn advance(&mut self) {

#[inline]
fn peek(&self) -> Option<char> {
```

**Step 3: Run benchmarks to verify improvement**

Run: `cargo bench -p nebula-expression`
Expected: No regression, possible slight improvement

**Step 4: Commit**

```bash
git add crates/nebula-expression/src/eval/mod.rs crates/nebula-expression/src/lexer/mod.rs
git commit -m "perf(nebula-expression): add inline hints to hot paths"
```

---

### Task 12: Use Arc::from for Interned Strings

**Files:**
- Modify: `crates/nebula-expression/src/parser/mod.rs`

**Step 1: Verify Arc<str> usage is consistent**

Check that all string interning uses `Arc::from()` pattern consistently.

**Step 2: Document the pattern**

Add comment in parser:
```rust
// Performance note: We use Arc<str> for all identifiers and property names
// to enable cheap cloning and reduce memory for repeated strings.
// The Arc::from(*name) pattern creates an owned Arc from the borrowed &str.
```

**Step 3: Commit**

```bash
git add crates/nebula-expression/src/parser/mod.rs
git commit -m "docs(nebula-expression): document Arc<str> interning pattern"
```

---

## Phase 5: Implement Lambda Support (filter/map/reduce)

### Task 13: Implement filter() with Lambda Support

**Files:**
- Modify: `crates/nebula-expression/src/builtins/array.rs`
- Modify: `crates/nebula-expression/src/builtins/mod.rs`
- Modify: `crates/nebula-expression/src/eval/mod.rs`

**Step 1: Write failing test for filter**

```rust
#[test]
fn test_filter_with_lambda() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::Array(nebula_value::Array::from_vec(vec![
        Value::integer(1),
        Value::integer(2),
        Value::integer(3),
        Value::integer(4),
        Value::integer(5),
    ])));
    
    let result = engine
        .evaluate("{{ $input | filter(x => x > 2) }}", &context)
        .unwrap();
    
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-expression --lib -- test_filter_with_lambda`
Expected: FAIL with "filter requires lambda support"

**Step 3: Modify builtins to receive AST for lambdas**

This requires architectural changes:
1. Pass `&[Expr]` to builtins that need lambdas, not just `&[Value]`
2. Or: Evaluate lambda inline in the evaluator before calling builtin

**Step 4: Implement filter with lambda support**

```rust
// New approach: evaluator handles lambdas specially for filter/map/reduce
// In Evaluator::call_function:
if name == "filter" && args.len() == 2 {
    if let Some(arr) = args[0].as_array() {
        // Lambda was already resolved - this won't work
        // Need to change architecture
    }
}
```

**Note:** This task is complex and may need to be broken into subtasks.

**Step 5: Run test to verify it passes**

Run: `cargo test -p nebula-expression -- test_filter_with_lambda`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/nebula-expression/src/
git commit -m "feat(nebula-expression): implement filter with lambda support"
```

---

### Task 14: Implement map() with Lambda Support

**Files:**
- Modify: `crates/nebula-expression/src/builtins/array.rs`

**Step 1: Write failing test**

```rust
#[test]
fn test_map_with_lambda() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::Array(nebula_value::Array::from_vec(vec![
        Value::integer(1),
        Value::integer(2),
        Value::integer(3),
    ])));
    
    let result = engine
        .evaluate("{{ $input | map(x => x * 2) }}", &context)
        .unwrap();
    
    let arr = result.as_array().unwrap();
    assert_eq!(arr.get(0).unwrap().as_integer(), Some(Integer::new(2)));
    assert_eq!(arr.get(1).unwrap().as_integer(), Some(Integer::new(4)));
    assert_eq!(arr.get(2).unwrap().as_integer(), Some(Integer::new(6)));
}
```

**Step 2: Implement map (follows same pattern as filter)**

**Step 3: Run tests**

Run: `cargo test -p nebula-expression -- test_map_with_lambda`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/nebula-expression/src/builtins/array.rs
git commit -m "feat(nebula-expression): implement map with lambda support"
```

---

### Task 15: Implement reduce() with Lambda Support

**Files:**
- Modify: `crates/nebula-expression/src/builtins/array.rs`

**Step 1: Write failing test**

```rust
#[test]
fn test_reduce_with_lambda() {
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::Array(nebula_value::Array::from_vec(vec![
        Value::integer(1),
        Value::integer(2),
        Value::integer(3),
        Value::integer(4),
    ])));
    
    // Sum all elements
    let result = engine
        .evaluate("{{ $input | reduce((acc, x) => acc + x, 0) }}", &context)
        .unwrap();
    
    assert_eq!(result.as_integer(), Some(Integer::new(10)));
}
```

**Note:** reduce requires 2-parameter lambda - parser may need update.

**Step 2: Implement reduce**

**Step 3: Run tests**

**Step 4: Commit**

```bash
git add crates/nebula-expression/src/
git commit -m "feat(nebula-expression): implement reduce with lambda support"
```

---

## Phase 6: Final Verification & Cleanup

### Task 16: Run Full CI Pipeline

**Step 1: Format check**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues

**Step 2: Clippy check**

Run: `cargo clippy -p nebula-expression --all-features -- -D warnings`
Expected: No errors

**Step 3: Full test suite**

Run: `cargo test -p nebula-expression --all-features`
Expected: All tests PASS

**Step 4: Doc generation**

Run: `cargo doc -p nebula-expression --no-deps`
Expected: No warnings

**Step 5: Commit any final fixes**

---

### Task 17: Update CHANGELOG

**Files:**
- Create/Modify: `crates/nebula-expression/CHANGELOG.md`

**Step 1: Document all changes**

```markdown
# Changelog

## [Unreleased]

### Fixed
- Fixed test type mismatches with Integer::new() wrapper
- Removed invalid `std` feature flag references
- Added ReDoS protection for regex patterns
- Added regex cache size limit (100 patterns max)

### Changed
- Optimized string trim to avoid allocation when unchanged
- Added inline hints to hot path functions
- Improved documentation coverage

### Added
- Implemented filter() with lambda support
- Implemented map() with lambda support
- Implemented reduce() with lambda support
```

**Step 2: Commit**

```bash
git add crates/nebula-expression/CHANGELOG.md
git commit -m "docs(nebula-expression): update changelog"
```

---

## Summary

| Phase | Tasks | Focus |
|-------|-------|-------|
| 1 | 1-5 | Fix compilation errors (Critical) |
| 2 | 6-7 | Code quality & cleanup |
| 3 | 8-9 | Security improvements |
| 4 | 10-12 | Advanced Rust patterns |
| 5 | 13-15 | Lambda support (filter/map/reduce) |
| 6 | 16-17 | Final verification |

**Estimated Tasks:** 17 bite-sized tasks
**Priority:** Phase 1 is critical - tests must compile first
