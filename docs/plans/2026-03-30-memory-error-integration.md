# Memory Error Integration Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace manual `Classify` impl in nebula-memory with derive macro, remove duplicate inherent methods, fix bugs, and add `PartialEq<&str>` to `ErrorCode`.

**Architecture:** Two crates change. nebula-error gets a non-breaking ergonomic addition (`PartialEq<&str>` for `ErrorCode`, rename `DeriveClassify` → `Classify` re-export). nebula-memory gets a breaking rewrite of its error type to use `#[derive(Classify)]`, removing duplicate inherent methods and fixing the `not_supported()` constructor bug.

**Tech Stack:** Rust 1.93, thiserror, nebula-error derive macro, nebula-error-macros proc-macro crate.

---

## Task 1: Add `PartialEq<&str>` to `ErrorCode` (nebula-error)

**Files:**
- Modify: `crates/error/src/code.rs`

**Step 1: Write the failing test**

Add to the existing `mod tests` block in `crates/error/src/code.rs`:

```rust
#[test]
fn error_code_eq_str() {
    let code = ErrorCode::new("MY_CODE");
    assert_eq!(code, "MY_CODE");
    assert_eq!("MY_CODE", code);
    assert_ne!(code, "OTHER");
}
```

**Step 2: Run test to verify it fails**

Run: `rtk cargo nextest run -p nebula-error -- error_code_eq_str`
Expected: FAIL — `PartialEq<&str>` not implemented

**Step 3: Implement `PartialEq<&str>` and reverse**

Add after the `impl fmt::Display for ErrorCode` block (line 76) in `crates/error/src/code.rs`:

```rust
impl PartialEq<&str> for ErrorCode {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<ErrorCode> for &str {
    fn eq(&self, other: &ErrorCode) -> bool {
        *self == other.as_str()
    }
}
```

**Step 4: Run test to verify it passes**

Run: `rtk cargo nextest run -p nebula-error -- error_code_eq_str`
Expected: PASS

**Step 5: Commit**

```bash
rtk git add crates/error/src/code.rs
rtk git commit -m "feat(error): add PartialEq<&str> for ErrorCode"
```

---

## Task 2: Rename `DeriveClassify` re-export to `Classify` (nebula-error)

**Files:**
- Modify: `crates/error/src/lib.rs`

**Context:** In Rust, proc-macro derives and traits live in different namespaces. `#[derive(Classify)]` resolves to the proc-macro, `use ... Classify` resolves to the trait. Serde does the same pattern (exports both trait `Serialize` and derive `Serialize`). The current `DeriveClassify` name is unnecessary.

**Step 1: Check no external consumers use `DeriveClassify`**

Run: `grep -r "DeriveClassify" crates/`
Expected: Only `crates/error/src/lib.rs` — no other crate uses the re-export yet (they all have manual impls).

**Step 2: Rename the re-export**

In `crates/error/src/lib.rs`, change line 57:

```rust
// Before:
pub use nebula_error_macros::Classify as DeriveClassify;

// After:
pub use nebula_error_macros::Classify;
```

**Step 3: Verify it compiles**

Run: `rtk cargo check -p nebula-error`
Expected: PASS — no namespace conflict between trait `Classify` and derive macro `Classify`.

**Step 4: Commit**

```bash
rtk git add crates/error/src/lib.rs
rtk git commit -m "refactor(error): rename DeriveClassify re-export to Classify"
```

---

## Task 3: Enable `derive` feature for nebula-error in nebula-memory (nebula-memory)

**Files:**
- Modify: `crates/memory/Cargo.toml`

**Step 1: Add derive feature**

In `crates/memory/Cargo.toml`, change line 79:

```toml
# Before:
nebula-error = { workspace = true }

# After:
nebula-error = { workspace = true, features = ["derive"] }
```

**Step 2: Verify it compiles**

Run: `rtk cargo check -p nebula-memory`
Expected: PASS

**Step 3: Commit**

```bash
rtk git add crates/memory/Cargo.toml
rtk git commit -m "chore(memory): enable nebula-error derive feature"
```

---

## Task 4: Replace manual Classify with derive macro (nebula-memory)

**Files:**
- Modify: `crates/memory/src/error.rs`

This is the main task. We will:
1. Add `#[derive(nebula_error::Classify)]` to `MemoryError`
2. Add `#[classify(...)]` attributes to each variant
3. Remove the manual `impl Classify` block
4. Remove the duplicate inherent `is_retryable()` and `code()` methods
5. Fix `not_supported()` constructor

**Step 1: Add derive and classify attributes, remove manual impl**

Replace the enum definition and remove the manual Classify impl. The new enum:

```rust
use nebula_error::ErrorCategory; // keep for tests if needed, remove ErrorCode import

/// Memory management errors
#[must_use = "errors should be handled"]
#[non_exhaustive]
#[derive(Error, Debug, Clone, nebula_error::Classify)]
pub enum MemoryError {
    // --- Allocation Errors ---
    #[classify(category = "internal", code = "MEM:ALLOC:FAILED")]
    #[error("Memory allocation failed: {size} bytes with {align} byte alignment")]
    AllocationFailed { size: usize, align: usize },

    #[classify(category = "internal", code = "MEM:ALLOC:LAYOUT")]
    #[error("Invalid memory layout: {reason}")]
    InvalidLayout { reason: Box<str> },

    #[classify(category = "internal", code = "MEM:ALLOC:OVERFLOW")]
    #[error("Size overflow during operation: {operation}")]
    SizeOverflow { operation: Box<str> },

    #[classify(category = "internal", code = "MEM:ALLOC:ALIGN")]
    #[error("Invalid alignment: {alignment}")]
    InvalidAlignment { alignment: usize },

    #[classify(category = "internal", code = "MEM:ALLOC:MAX")]
    #[error("Allocation exceeds maximum size: {size} bytes (max: {max_size})")]
    ExceedsMaxSize { size: usize, max_size: usize },

    // --- Pool Errors ---
    #[classify(category = "exhausted", code = "MEM:POOL:EXHAUSTED")]
    #[error("Memory pool '{pool_id}' exhausted (capacity: {capacity})")]
    PoolExhausted { pool_id: Box<str>, capacity: usize },

    #[classify(category = "validation", code = "MEM:CONFIG:INVALID")]
    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: Box<str> },

    // --- Arena Errors ---
    #[classify(category = "exhausted", code = "MEM:ARENA:EXHAUSTED")]
    #[error("Arena '{arena_id}' exhausted: requested {requested} bytes, available {available}")]
    ArenaExhausted {
        arena_id: Box<str>,
        requested: usize,
        available: usize,
    },

    // --- Cache Errors ---
    #[classify(category = "not_found", code = "MEM:CACHE:MISS", retryable = true)]
    #[error("Cache miss for key: {key}")]
    CacheMiss { key: Box<str> },

    #[classify(category = "exhausted", code = "MEM:CACHE:OVERFLOW")]
    #[error("Cache overflow: {current} bytes used, {max} bytes maximum")]
    CacheOverflow { current: usize, max: usize },

    #[classify(category = "validation", code = "MEM:CACHE:KEY")]
    #[error("Invalid cache key: {reason}")]
    InvalidCacheKey { reason: Box<str> },

    // --- Budget Errors ---
    #[classify(category = "exhausted", code = "MEM:BUDGET:EXCEEDED")]
    #[error("Memory budget exceeded: {used} bytes used, {limit} bytes limit")]
    BudgetExceeded { used: usize, limit: usize },

    // --- System Errors ---
    #[classify(category = "internal", code = "MEM:SYSTEM:CORRUPTION")]
    #[error("Memory corruption detected in {component}: {details}")]
    Corruption {
        component: Box<str>,
        details: Box<str>,
    },

    #[classify(category = "internal", code = "MEM:SYSTEM:CONCURRENT")]
    #[error("Concurrent access error: {details}")]
    ConcurrentAccess { details: Box<str> },

    #[classify(category = "internal", code = "MEM:SYSTEM:STATE")]
    #[error("Invalid state: {reason}")]
    InvalidState { reason: Box<str> },

    #[classify(category = "internal", code = "MEM:SYSTEM:INIT")]
    #[error("Initialization failed: {reason}")]
    InitializationFailed { reason: Box<str> },

    // --- Feature Support Errors ---
    #[classify(category = "unsupported", code = "MEM:FEATURE:UNSUPPORTED")]
    #[error("Feature not supported: {feature}{}", context.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    NotSupported {
        feature: &'static str,
        context: Option<Box<str>>,
    },

    // --- General Errors ---
    #[classify(category = "not_found", code = "MEM:NOT_FOUND")]
    #[error("Operation not found: {reason}")]
    NotFound { reason: Box<str> },

    #[classify(category = "validation", code = "MEM:INVALID_OP")]
    #[error("Invalid operation: {reason}")]
    InvalidOperation { reason: Box<str> },
}
```

**Step 2: Remove the duplicate inherent methods and manual Classify impl**

Delete these blocks from `impl MemoryError`:
- `pub fn is_retryable(&self) -> bool` (lines 94-106)
- `pub fn code(&self) -> &'static str` (lines 108-132)

Delete the entire `impl Classify for MemoryError` block (lines 361-391).

Update the import at the top of the file:
```rust
// Before:
use nebula_error::{Classify, ErrorCategory, ErrorCode};

// After:
use nebula_error::ErrorCategory;
```

(`Classify` and `ErrorCode` are no longer needed — the derive macro uses fully-qualified paths.)

**Step 3: Fix `not_supported()` constructor**

Change the `not_supported()` method from:

```rust
pub fn not_supported(operation: &str) -> Self {
    Self::InvalidState {
        reason: format!("operation not supported: {operation}").into_boxed_str(),
    }
}
```

To:

```rust
pub fn not_supported(feature: &'static str) -> Self {
    Self::NotSupported {
        feature,
        context: None,
    }
}
```

Note: parameter type changes from `&str` to `&'static str` because `NotSupported::feature` is `&'static str`. Check callers — if any pass non-static strings, they'll need updating.

**Step 4: Verify it compiles**

Run: `rtk cargo check -p nebula-memory`
Expected: PASS

**Step 5: Don't commit yet — fix tests first (Task 5)**

---

## Task 5: Update tests (nebula-memory)

**Files:**
- Modify: `crates/memory/src/error.rs` (unit tests)
- Modify: `crates/memory/tests/stress_and_edge_cases.rs` (integration tests)

**Step 1: Update unit tests in `error.rs`**

The `test_error_codes` test calls `error.code()` which now returns `ErrorCode`, not `&'static str`. Thanks to Task 1's `PartialEq<&str>`, `assert_eq!` still works. But we need to bring `Classify` into scope.

Replace the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_error::Classify;

    #[test]
    fn test_memory_error_creation() {
        let error = MemoryError::allocation_failed(1024, 8);
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("1024"));
    }

    #[test]
    fn test_error_with_layout() {
        let layout = Layout::new::<u64>();
        let error = MemoryError::allocation_failed_with_layout(layout);
        assert!(error.to_string().contains(&layout.size().to_string()));
    }

    #[test]
    fn test_convenience_constructors() {
        let alloc_error = MemoryError::allocation_failed(1024, 8);
        let pool_error = MemoryError::pool_exhausted("test_pool", 100);
        let cache_error = MemoryError::cache_miss("test_key");

        assert!(!alloc_error.to_string().is_empty());
        assert!(!pool_error.to_string().is_empty());
        assert!(!cache_error.to_string().is_empty());
    }

    #[test]
    fn test_arena_errors() {
        let error = MemoryError::arena_exhausted("test_arena", 1024, 512);
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("test_arena"));
    }

    #[test]
    fn test_budget_errors() {
        let error = MemoryError::budget_exceeded(2048, 1024);
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("2048"));
    }

    #[test]
    fn test_corruption_errors() {
        let error = MemoryError::corruption("allocator", "invalid pointer");
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("allocator"));
    }

    #[test]
    fn test_error_codes() {
        let error = MemoryError::allocation_failed(1024, 8);
        assert_eq!(error.code(), "MEM:ALLOC:FAILED");

        let error = MemoryError::pool_exhausted("test", 100);
        assert_eq!(error.code(), "MEM:POOL:EXHAUSTED");
    }

    #[test]
    fn test_retryable() {
        assert!(MemoryError::pool_exhausted("test", 100).is_retryable());
        assert!(!MemoryError::invalid_alignment(8).is_retryable());
    }
}
```

Key change: added `use nebula_error::Classify;` — the `code()` and `is_retryable()` methods are now on the trait, not inherent. Method resolution requires the trait to be in scope.

**Step 2: Update integration tests in `tests/stress_and_edge_cases.rs`**

The `error_variants` module already imports `use nebula_error::Classify;` (line 482), so trait methods are in scope. The `.code()` comparisons will work thanks to `PartialEq<&str>`.

However, the `.is_retryable()` calls were previously calling the inherent method. Now they call the trait method. Behavior should be identical because:
- `Exhausted` category is default-retryable → PoolExhausted, ArenaExhausted, CacheOverflow, BudgetExceeded ✓
- `CacheMiss` has `retryable = true` override ✓
- `Internal` is not default-retryable → AllocationFailed ✓
- `Validation` is not default-retryable → InvalidConfig ✓

No changes needed to integration tests — verify by running them.

**Step 3: Check for callers of removed `not_supported()`**

Run: `grep -rn "not_supported" crates/memory/src/`

If any callers pass non-`'static` strings (e.g., format results), they need updating. Likely callers use string literals which are `&'static str`.

**Step 4: Run all tests**

Run: `rtk cargo nextest run -p nebula-memory`
Expected: PASS

**Step 5: Commit**

```bash
rtk git add crates/memory/src/error.rs crates/memory/Cargo.toml
rtk git commit -m "feat(memory): replace manual Classify impl with derive macro

BREAKING: removed inherent is_retryable() and code() methods from MemoryError.
Use nebula_error::Classify trait methods instead (import Classify into scope).
Fixed not_supported() constructor to use NotSupported variant."
```

---

## Task 6: Verify nebula-expression still compiles

**Files:**
- Check: `crates/expression/src/error.rs` (From<MemoryError> impl)
- Check: `crates/expression/src/engine.rs` (MemoryError::invalid_layout usage)

**Step 1: Run check**

Run: `rtk cargo check -p nebula-expression`
Expected: PASS — expression only uses `From<MemoryError>` (calls `.to_string()`) and `MemoryError::invalid_layout()` constructor (unchanged).

**Step 2: Run full workspace**

Run: `rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace`
Expected: PASS

**Step 3: If anything fails, fix it**

---

## Task 7: Update context file

**Files:**
- Modify: `.claude/crates/memory.md`
- Modify: `.claude/crates/error.md`

**Step 1: Update memory.md**

Update the error-related section:
- Note: `MemoryError` uses `#[derive(Classify)]` — no manual impl
- Remove mention of inherent `is_retryable()` and `code()` methods
- Note: `not_supported()` constructor now uses `NotSupported` variant
- Note: consumers must `use nebula_error::Classify` to call `.code()` / `.is_retryable()`

**Step 2: Update error.md**

- Note: `DeriveClassify` renamed to `Classify` re-export
- Note: `ErrorCode` now implements `PartialEq<&str>`
- Note: nebula-memory is first crate using derive macro

**Step 3: Commit**

```bash
rtk git add .claude/crates/memory.md .claude/crates/error.md
rtk git commit -m "docs: update memory and error context files"
```
