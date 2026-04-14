# Phase 10: ErrorCode & ActionResultExt — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add machine-readable ErrorCode to ActionError and ergonomic ActionResultExt trait for action authors.

**Architecture:** ErrorCode is a `#[non_exhaustive]` enum added as `Option<ErrorCode>` field to `Retryable` and `Fatal` variants. `ActionResultExt` is a trait on `Result<T, E: Into<anyhow::Error>>` providing `.retryable()?` and `.fatal()?` shorthand. Error field changes from `String` to `Arc<anyhow::Error>` for full error chain preservation while keeping `Clone`.

**Tech Stack:** Rust 1.94, `anyhow`, `thiserror`, `serde`, `serde_json`

---

### Task 1: Add ErrorCode enum

**Files:**
- Modify: `crates/action/src/error.rs`
- Test: `crates/action/src/error.rs` (inline mod tests)

**Step 1: Write the failing test**

Add to `mod tests` in `error.rs`:

```rust
#[test]
fn error_code_default_is_none() {
    let err = ActionError::retryable("timeout");
    assert!(err.error_code().is_none());
}

#[test]
fn error_code_rate_limited() {
    let err = ActionError::retryable_with_code("rate limited", ErrorCode::RateLimited);
    assert_eq!(err.error_code(), Some(&ErrorCode::RateLimited));
    assert!(err.is_retryable());
}

#[test]
fn error_code_auth_expired() {
    let err = ActionError::fatal_with_code("token expired", ErrorCode::AuthExpired);
    assert_eq!(err.error_code(), Some(&ErrorCode::AuthExpired));
    assert!(err.is_fatal());
}

#[test]
fn error_code_serializes() {
    let code = ErrorCode::RateLimited;
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, "\"RateLimited\"");
    let back: ErrorCode = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ErrorCode::RateLimited);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo nextest run -p nebula-action -- error_code`
Expected: FAIL — `ErrorCode` not defined

**Step 3: Implement ErrorCode enum**

Add before `ActionError` in `error.rs`:

```rust
/// Machine-readable error classification for engine retry decisions.
///
/// Engine can match on these codes to make smarter retry choices
/// (e.g., `RateLimited` → respect Retry-After header, `AuthExpired` → refresh credential).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Remote API returned 429 Too Many Requests.
    RateLimited,
    /// Concurrent modification conflict (e.g., optimistic lock failure).
    Conflict,
    /// Credential expired — engine may refresh and retry.
    AuthExpired,
    /// Remote service is down or unreachable.
    UpstreamUnavailable,
    /// Remote call timed out.
    UpstreamTimeout,
    /// Input data invalid for the remote service (not action validation).
    InvalidInput,
    /// Usage quota exhausted (e.g., API call limit).
    QuotaExhausted,
    /// Action panicked during execution (caught by runtime).
    ActionPanicked,
}
```

**Step 4: Run test to verify it passes**

Run: `cargo nextest run -p nebula-action -- error_code`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/action/src/error.rs
git commit -m "feat(action): add ErrorCode enum for machine-readable error classification"
```

---

### Task 2: Change ActionError error field from String to Arc\<anyhow::Error\>

**Files:**
- Modify: `crates/action/src/error.rs`
- Modify: `crates/action/Cargo.toml` (add `anyhow` dependency)
- Test: `crates/action/src/error.rs` (inline mod tests)

**Step 1: Add anyhow dependency**

In `crates/action/Cargo.toml`, add to `[dependencies]`:
```toml
anyhow = "1"
```

**Step 2: Write the failing test**

```rust
#[test]
fn retryable_preserves_error_chain() {
    let io_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "connection timeout");
    let err = ActionError::retryable(io_err);
    // Display shows the message
    assert!(err.to_string().contains("connection timeout"));
    // Error chain preserved (not flattened to string)
    assert!(err.is_retryable());
}

#[test]
fn retryable_clone_works() {
    let err = ActionError::retryable("test error");
    let cloned = err.clone();
    assert_eq!(err.to_string(), cloned.to_string());
}
```

**Step 3: Migrate ActionError variants**

Change `Retryable` and `Fatal` in `error.rs`:

```rust
use std::sync::Arc;

#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum ActionError {
    #[error("retryable: {error}")]
    Retryable {
        /// Full error chain wrapped in Arc for Clone support.
        error: Arc<anyhow::Error>,
        /// Machine-readable error code for engine decisions.
        code: Option<ErrorCode>,
        /// Suggested delay before retry (engine may override).
        backoff_hint: Option<Duration>,
        /// Partial result produced before failure.
        partial_output: Option<serde_json::Value>,
    },

    #[error("fatal: {error}")]
    Fatal {
        /// Full error chain wrapped in Arc for Clone support.
        error: Arc<anyhow::Error>,
        /// Machine-readable error code for engine decisions.
        code: Option<ErrorCode>,
        /// Optional structured details about the failure.
        details: Option<serde_json::Value>,
    },
    // ... Validation, SandboxViolation, Cancelled, DataLimitExceeded unchanged
}
```

**Step 4: Update all factory methods**

Update existing factory methods to accept `impl Into<anyhow::Error>` and wrap in Arc:

```rust
impl ActionError {
    /// Create a retryable error from any error type.
    pub fn retryable(error: impl Into<anyhow::Error>) -> Self {
        Self::Retryable {
            error: Arc::new(error.into()),
            code: None,
            backoff_hint: None,
            partial_output: None,
        }
    }

    /// Create a retryable error with a backoff hint.
    pub fn retryable_with_backoff(
        error: impl Into<anyhow::Error>,
        backoff: Duration,
    ) -> Self {
        Self::Retryable {
            error: Arc::new(error.into()),
            code: None,
            backoff_hint: Some(backoff),
            partial_output: None,
        }
    }

    /// Create a retryable error with an error code.
    pub fn retryable_with_code(
        error: impl Into<anyhow::Error>,
        code: ErrorCode,
    ) -> Self {
        Self::Retryable {
            error: Arc::new(error.into()),
            code: Some(code),
            backoff_hint: None,
            partial_output: None,
        }
    }

    /// Create a retryable error with partial output from before the failure.
    pub fn retryable_with_partial(
        error: impl Into<anyhow::Error>,
        partial: serde_json::Value,
    ) -> Self {
        Self::Retryable {
            error: Arc::new(error.into()),
            code: None,
            backoff_hint: None,
            partial_output: Some(partial),
        }
    }

    /// Create a fatal (non-retryable) error.
    pub fn fatal(error: impl Into<anyhow::Error>) -> Self {
        Self::Fatal {
            error: Arc::new(error.into()),
            code: None,
            details: None,
        }
    }

    /// Create a fatal error with structured details.
    pub fn fatal_with_details(
        error: impl Into<anyhow::Error>,
        details: serde_json::Value,
    ) -> Self {
        Self::Fatal {
            error: Arc::new(error.into()),
            code: None,
            details: Some(details),
        }
    }

    /// Create a fatal error with an error code.
    pub fn fatal_with_code(
        error: impl Into<anyhow::Error>,
        code: ErrorCode,
    ) -> Self {
        Self::Fatal {
            error: Arc::new(error.into()),
            code: Some(code),
            details: None,
        }
    }

    /// Get the error code, if any.
    pub fn error_code(&self) -> Option<&ErrorCode> {
        match self {
            Self::Retryable { code, .. } | Self::Fatal { code, .. } => code.as_ref(),
            _ => None,
        }
    }
}
```

**Step 5: Fix all compilation errors across the workspace**

Run: `cargo check --workspace`

Any code matching on `ActionError::Retryable { error, .. }` or `ActionError::Fatal { error, .. }` will need to handle `Arc<anyhow::Error>` instead of `String`. Common pattern:

```rust
// Before: error: String
// After: error: Arc<anyhow::Error>
// Display trait works the same way — .to_string() still works
```

Also fix any code constructing ActionError directly (instead of using factory methods) to include the new `code: None` field.

**Step 6: Run tests**

Run: `cargo nextest run -p nebula-action`
Expected: PASS

Run: `cargo clippy --workspace -- -D warnings`
Expected: PASS (check for unused imports, etc.)

**Step 7: Commit**

```bash
git add crates/action/src/error.rs crates/action/Cargo.toml
# Also add any other files that needed fixes
git commit -m "feat(action): change ActionError to Arc<anyhow::Error> + ErrorCode field

Preserves full error chain while maintaining Clone via Arc.
ErrorCode provides machine-readable classification for engine retry decisions."
```

---

### Task 3: Add ActionResultExt trait

**Files:**
- Create: `crates/action/src/ext.rs`
- Modify: `crates/action/src/lib.rs` (add module + re-export)
- Modify: `crates/action/src/prelude.rs` (re-export)
- Test: `crates/action/src/ext.rs` (inline mod tests)

**Step 1: Write the failing test**

Create `crates/action/src/ext.rs` with tests first:

```rust
//! Extension traits for ergonomic error conversion in actions.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ActionError, ErrorCode};

    #[test]
    fn retryable_converts_io_error() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        let action_result: Result<(), ActionError> = result.retryable();
        assert!(action_result.is_err());
        assert!(action_result.unwrap_err().is_retryable());
    }

    #[test]
    fn fatal_converts_io_error() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "corrupt data",
        ));
        let action_result: Result<(), ActionError> = result.fatal();
        assert!(action_result.is_err());
        assert!(action_result.unwrap_err().is_fatal());
    }

    #[test]
    fn retryable_with_code_converts() {
        let result: Result<i32, &str> = Err("rate limited");
        let action_result = result.retryable_with_code(ErrorCode::RateLimited);
        let err = action_result.unwrap_err();
        assert_eq!(err.error_code(), Some(&ErrorCode::RateLimited));
    }

    #[test]
    fn ok_passes_through() {
        let result: Result<i32, std::io::Error> = Ok(42);
        let action_result: Result<i32, ActionError> = result.retryable();
        assert_eq!(action_result.unwrap(), 42);
    }

    #[test]
    fn chaining_works() {
        fn do_work() -> Result<String, ActionError> {
            let value: i32 = "42".parse().fatal()?;
            let text = std::str::from_utf8(b"hello").retryable()?;
            Ok(format!("{text}:{value}"))
        }
        assert_eq!(do_work().unwrap(), "hello:42");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo nextest run -p nebula-action -- ext`
Expected: FAIL — module not found

**Step 3: Implement ActionResultExt**

Add to `crates/action/src/ext.rs` (above the tests module):

```rust
//! Extension traits for ergonomic error conversion in actions.
//!
//! Provides `.retryable()?` and `.fatal()?` on any `Result<T, E>`
//! where `E: Into<anyhow::Error>`, eliminating verbose `.map_err(...)` chains.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//!
//! async fn my_action(ctx: &ActionContext) -> Result<Value, ActionError> {
//!     let response = client.get(url).await.retryable()?;
//!     let data: MyData = response.json().await.fatal()?;
//!     Ok(ActionResult::success(serde_json::to_value(data).fatal()?))
//! }
//! ```

use crate::error::{ActionError, ErrorCode};

/// Extension trait for converting any `Result<T, E>` into `Result<T, ActionError>`.
///
/// Provides ergonomic `.retryable()?` and `.fatal()?` methods that wrap
/// the error in `ActionError::Retryable` or `ActionError::Fatal` respectively.
pub trait ActionResultExt<T> {
    /// Convert error to `ActionError::Retryable` (transient — engine may retry).
    fn retryable(self) -> Result<T, ActionError>;

    /// Convert error to `ActionError::Fatal` (permanent — never retry).
    fn fatal(self) -> Result<T, ActionError>;

    /// Convert error to `ActionError::Retryable` with a specific error code.
    fn retryable_with_code(self, code: ErrorCode) -> Result<T, ActionError>;

    /// Convert error to `ActionError::Fatal` with a specific error code.
    fn fatal_with_code(self, code: ErrorCode) -> Result<T, ActionError>;
}

impl<T, E> ActionResultExt<T> for Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn retryable(self) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::retryable(e))
    }

    fn fatal(self) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::fatal(e))
    }

    fn retryable_with_code(self, code: ErrorCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::retryable_with_code(e, code))
    }

    fn fatal_with_code(self, code: ErrorCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::fatal_with_code(e, code))
    }
}
```

**Step 4: Register module and re-export**

In `crates/action/src/lib.rs`, add:
```rust
mod ext;
pub use ext::ActionResultExt;
```

In `crates/action/src/prelude.rs`, add:
```rust
pub use crate::ext::ActionResultExt;
```

**Step 5: Run tests**

Run: `cargo nextest run -p nebula-action -- ext`
Expected: PASS

Run: `cargo fmt && cargo clippy -p nebula-action -- -D warnings`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/action/src/ext.rs crates/action/src/lib.rs crates/action/src/prelude.rs
git commit -m "feat(action): add ActionResultExt for ergonomic error conversion

Provides .retryable()? and .fatal()? on any Result<T, E: Into<anyhow::Error>>.
Eliminates verbose .map_err(|e| ActionError::retryable(e.to_string())) patterns."
```

---

### Task 4: Full workspace check + contract tests

**Files:**
- Modify: `crates/action/tests/contracts.rs` (if error serialization contracts exist)

**Step 1: Run full workspace build**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings`
Expected: PASS (fix any warnings in downstream crates that use ActionError)

**Step 2: Run full test suite**

Run: `cargo nextest run --workspace`
Expected: PASS

**Step 3: Run doc tests**

Run: `cargo test --workspace --doc`
Expected: PASS

**Step 4: Update contract tests if needed**

If `crates/action/tests/contracts.rs` has serialization snapshots for `ActionError`, update them to include the new `code` field.

**Step 5: Commit any fixes**

```bash
git add -A
git commit -m "fix(action): update downstream crates for ActionError Arc<anyhow::Error> + ErrorCode"
```

---

### Task 5: Update .claude/crates/action.md

**Files:**
- Modify: `.claude/crates/action.md`

**Step 1: Update context file**

Add to Key Decisions:
- `ErrorCode` enum on `ActionError::Retryable` and `Fatal` — machine-readable classification for engine retry decisions.
- `ActionResultExt` trait — `.retryable()?` and `.fatal()?` ergonomic conversion.
- Error field changed from `String` to `Arc<anyhow::Error>` — preserves error chain, Clone via Arc.

Update reviewed date.

**Step 2: Commit**

```bash
git add .claude/crates/action.md
git commit -m "docs(action): update context file for ErrorCode + ActionResultExt"
```

---

## Summary

| Task | What | Effort |
|------|------|--------|
| 1 | ErrorCode enum | 30 min |
| 2 | Arc\<anyhow::Error\> migration + code field | 1-2 hours |
| 3 | ActionResultExt trait | 30 min |
| 4 | Workspace validation | 30 min |
| 5 | Context file update | 10 min |

**Total estimated effort: 3-4 hours (1 day)**

**Exit criteria from roadmap:**
- [x] `ErrorCode` enum with 8 variants, `#[non_exhaustive]`
- [x] `code: Option<ErrorCode>` on Retryable and Fatal
- [x] `ActionResultExt` with `.retryable()`, `.fatal()`, `_with_code()` variants
- [x] Full workspace builds and tests pass
- [x] Context file updated
