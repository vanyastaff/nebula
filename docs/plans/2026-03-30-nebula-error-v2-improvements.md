# nebula-error v2 — Research-Driven Improvements

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Upgrade nebula-error based on competitive analysis of 15 workflow engines and 90+ projects to close the gaps identified in pain clusters 5, 6, 8, and 10.

**Architecture:** Six independent improvement areas, each additive (no breaking changes to existing API). New types and methods extend the existing Classify + NebulaError<E> foundation. Feature flags isolate optional dependencies.

**Tech Stack:** Rust 1.93, thiserror (dev), serde (optional), nebula-error-macros (optional), http (optional, new)

---

## Context from Research

The `docs/research/2026-03-29-fundamental-improvements.md` identified these pain clusters directly relevant to nebula-error:

| Cluster | Problem | Current Gap | This Plan |
|---------|---------|-------------|-----------|
| **8: Silent Failures** | Errors lose execution context (node_id, correlation_id, attempt) | NebulaError has context_chain (strings only) | Task 1: Typed execution context detail types |
| **6: Retry Inadequacies** | No conditional retry by error kind; RateLimit not retryable by default | `is_retryable()` is boolean; RateLimit excluded | Task 2: RateLimit retryable + error-kind predicate support |
| **10: Error Routing** | Binary success/fail model, no structured routing info | No routing metadata on errors | Task 3: ErrorRoute detail type for error-edge routing |
| **5: Data Passing** | Silent type casts (Flyte#4505) | No type mismatch detail | Task 4: TypeMismatch detail type |
| **convert.rs empty** | HTTP/gRPC status mapping planned but unimplemented | Placeholder file | Task 5: ErrorCategory → HTTP status code mapping |
| **DX: NebulaError ergonomics** | Consumers need map_inner, serde for NebulaError | Missing utility methods | Task 6: Ergonomic extensions |

---

## Task 1: Execution Context Detail Types

Addresses **Cluster 8 (Silent Failures)** — "каждая ошибка несёт: original error, node_id, correlation_id, attempt_number"

**Files:**
- Modify: `crates/error/src/detail_types.rs`
- Test: `crates/error/src/detail_types.rs` (inline mod tests)

### Step 1: Write the failing test

Add to the `mod tests` block in `detail_types.rs`:

```rust
#[test]
fn execution_context_stored_and_retrieved() {
    let mut details = ErrorDetails::new();
    details.insert(ExecutionContext {
        node_id: Some("http-fetch-1".into()),
        workflow_id: Some("wf-daily-report".into()),
        correlation_id: Some("req-abc-123".into()),
        attempt: Some(2),
    });

    let ctx = details.get::<ExecutionContext>().unwrap();
    assert_eq!(ctx.node_id.as_deref(), Some("http-fetch-1"));
    assert_eq!(ctx.attempt, Some(2));
}
```

### Step 2: Run test to verify it fails

```bash
rtk cargo nextest run -p nebula-error execution_context_stored
```

Expected: FAIL — `ExecutionContext` not defined.

### Step 3: Write the implementation

Add to `detail_types.rs` before the tests module:

```rust
/// Execution context identifying where in a workflow an error occurred.
///
/// Attach this to errors that originate during workflow execution so
/// that error handlers, loggers, and monitoring can correlate failures
/// back to specific nodes and runs.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, ExecutionContext};
///
/// let mut details = ErrorDetails::new();
/// details.insert(ExecutionContext {
///     node_id: Some("http-fetch-1".into()),
///     workflow_id: Some("wf-daily-report".into()),
///     correlation_id: Some("req-abc-123".into()),
///     attempt: Some(2),
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    /// The node that produced this error, if known.
    pub node_id: Option<String>,
    /// The workflow run that this error belongs to.
    pub workflow_id: Option<String>,
    /// A correlation ID for distributed tracing (e.g. OTel trace ID).
    pub correlation_id: Option<String>,
    /// The retry attempt number (1-based), if this is a retried operation.
    pub attempt: Option<u32>,
}

impl ErrorDetail for ExecutionContext {}
```

### Step 4: Add re-export in `lib.rs`

Add `ExecutionContext` to the `pub use detail_types::` line.

### Step 5: Run test to verify it passes

```bash
rtk cargo nextest run -p nebula-error execution_context_stored
```

Expected: PASS

### Step 6: Commit

```bash
rtk git add crates/error/src/detail_types.rs crates/error/src/lib.rs
rtk git commit -m "feat(error): add ExecutionContext detail type for workflow tracing"
```

---

## Task 2: RateLimit Default Retryable + Error-Kind Predicate

Addresses **Cluster 6 (Retry Inadequacies)** — "retry только transient errors; RateLimit should be retryable"

The research shows that across Temporal, Vector, Airflow, and Flyte, `RateLimit` errors are universally treated as retriable (with backoff). Currently `ErrorCategory::RateLimit` returns `false` from `is_default_retryable()`.

Also adds a predicate-based classification helper for the resilience layer to filter errors by category/severity without implementing custom logic.

**Files:**
- Modify: `crates/error/src/category.rs`
- Modify: `crates/error/src/traits.rs`
- Test: inline mod tests in both files

### Step 1: Write the failing test for RateLimit retryable

Add to `category.rs` tests:

```rust
#[test]
fn rate_limit_is_default_retryable() {
    assert!(ErrorCategory::RateLimit.is_default_retryable());
}
```

### Step 2: Run test to verify it fails

```bash
rtk cargo nextest run -p nebula-error rate_limit_is_default_retryable
```

Expected: FAIL — currently returns `false`.

### Step 3: Fix `is_default_retryable` to include `RateLimit`

In `category.rs`, change:

```rust
pub const fn is_default_retryable(&self) -> bool {
    matches!(self, Self::Timeout | Self::Exhausted | Self::External | Self::RateLimit)
}
```

Also move `RateLimit` from `not_client` to `not_server` list... wait, actually `RateLimit` is already NOT in `is_client_error` and NOT in `is_server_error`. That's a classification gap too. Rate limiting is arguably a server-side concern (the server is protecting itself). But let's keep it neutral — it's neither strictly client nor server. Leave `is_client_error` and `is_server_error` unchanged.

### Step 4: Fix the existing test `client_errors_are_correct`

The existing test in `category.rs` already has `RateLimit` in `not_client`, which is correct. But we need to update the test `rate_limit_is_not_retryable` → remove it (it would now be wrong).

Wait — the existing test is `validation_is_not_retryable` and there's no explicit `rate_limit_is_not_retryable`. But the `not_found_is_not_retryable` exists. Good — no existing test to update.

### Step 5: Run test to verify it passes

```bash
rtk cargo nextest run -p nebula-error rate_limit_is_default_retryable
```

Expected: PASS

### Step 6: Write the ErrorClassifier helper

Add to `traits.rs`:

```rust
/// A predicate-based error classifier for filtering errors by category.
///
/// Used by the resilience layer to decide which errors to retry, route,
/// or escalate without requiring a full `Classify` implementation.
///
/// # Examples
///
/// ```
/// use nebula_error::{Classify, ErrorCategory, ErrorCode, ErrorClassifier, codes};
///
/// let transient_only = ErrorClassifier::new(|cat| matches!(
///     cat,
///     ErrorCategory::Timeout | ErrorCategory::RateLimit | ErrorCategory::External
/// ));
///
/// struct TimeoutErr;
/// impl Classify for TimeoutErr {
///     fn category(&self) -> ErrorCategory { ErrorCategory::Timeout }
///     fn code(&self) -> ErrorCode { codes::TIMEOUT.clone() }
/// }
///
/// assert!(transient_only.matches(&TimeoutErr));
/// ```
pub struct ErrorClassifier {
    predicate: Box<dyn Fn(ErrorCategory) -> bool + Send + Sync>,
}

impl ErrorClassifier {
    /// Creates a classifier from a category predicate.
    pub fn new(predicate: impl Fn(ErrorCategory) -> bool + Send + Sync + 'static) -> Self {
        Self {
            predicate: Box::new(predicate),
        }
    }

    /// Returns `true` if the error's category matches the predicate.
    pub fn matches(&self, error: &impl Classify) -> bool {
        (self.predicate)(error.category())
    }

    /// A built-in classifier that matches all default-retryable categories.
    pub fn retryable() -> Self {
        Self::new(|cat| cat.is_default_retryable())
    }

    /// A built-in classifier that matches all client errors.
    pub fn client_errors() -> Self {
        Self::new(|cat| cat.is_client_error())
    }

    /// A built-in classifier that matches all server errors.
    pub fn server_errors() -> Self {
        Self::new(|cat| cat.is_server_error())
    }
}

impl std::fmt::Debug for ErrorClassifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErrorClassifier").finish_non_exhaustive()
    }
}
```

### Step 7: Write test for ErrorClassifier

```rust
#[test]
fn error_classifier_retryable_matches_timeout() {
    let classifier = ErrorClassifier::retryable();
    let err = MinimalError {
        cat: ErrorCategory::Timeout,
    };
    assert!(classifier.matches(&err));
}

#[test]
fn error_classifier_retryable_rejects_validation() {
    let classifier = ErrorClassifier::retryable();
    let err = MinimalError {
        cat: ErrorCategory::Validation,
    };
    assert!(!classifier.matches(&err));
}

#[test]
fn error_classifier_custom_predicate() {
    let only_auth = ErrorClassifier::new(|cat| {
        matches!(cat, ErrorCategory::Authentication | ErrorCategory::Authorization)
    });
    let auth = MinimalError {
        cat: ErrorCategory::Authentication,
    };
    let timeout = MinimalError {
        cat: ErrorCategory::Timeout,
    };
    assert!(only_auth.matches(&auth));
    assert!(!only_auth.matches(&timeout));
}
```

### Step 8: Add re-export in `lib.rs`

Add `ErrorClassifier` to `pub use traits::`.

### Step 9: Run all tests

```bash
rtk cargo nextest run -p nebula-error
```

Expected: ALL PASS

### Step 10: Commit

```bash
rtk git add crates/error/src/category.rs crates/error/src/traits.rs crates/error/src/lib.rs
rtk git commit -m "feat(error): make RateLimit retryable by default, add ErrorClassifier"
```

---

## Task 3: ErrorRoute Detail Type for Error-Edge Routing

Addresses **Cluster 10 (Error Routing)** — "Error edges для explicit routing + CallErrorKind dispatch"

When nebula-engine implements error edges (DAG edges of type `ErrorEdge`), errors need to carry routing hints — which error handler path to take. This detail type provides that metadata.

**Files:**
- Modify: `crates/error/src/detail_types.rs`
- Modify: `crates/error/src/lib.rs`
- Test: inline in `detail_types.rs`

### Step 1: Write the failing test

```rust
#[test]
fn error_route_stored_and_retrieved() {
    let mut details = ErrorDetails::new();
    details.insert(ErrorRoute {
        suggested_handler: Some("retry-with-backoff".into()),
        dead_letter: false,
    });

    let route = details.get::<ErrorRoute>().unwrap();
    assert_eq!(route.suggested_handler.as_deref(), Some("retry-with-backoff"));
    assert!(!route.dead_letter);
}

#[test]
fn error_route_dead_letter() {
    let mut details = ErrorDetails::new();
    details.insert(ErrorRoute {
        suggested_handler: None,
        dead_letter: true,
    });

    let route = details.get::<ErrorRoute>().unwrap();
    assert!(route.dead_letter);
}
```

### Step 2: Run test to verify it fails

```bash
rtk cargo nextest run -p nebula-error error_route_stored
```

Expected: FAIL

### Step 3: Write the implementation

Add to `detail_types.rs`:

```rust
/// Routing hint for error-edge traversal in workflow DAGs.
///
/// When a node fails and the DAG has error edges, this detail tells
/// the engine which error handler to route to, or whether the error
/// should go to a dead letter queue.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, ErrorRoute};
///
/// let mut details = ErrorDetails::new();
/// details.insert(ErrorRoute {
///     suggested_handler: Some("alert-oncall".into()),
///     dead_letter: false,
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorRoute {
    /// Name/ID of the suggested error handler node.
    pub suggested_handler: Option<String>,
    /// Whether this error should be routed to the dead letter queue.
    pub dead_letter: bool,
}

impl ErrorDetail for ErrorRoute {}
```

### Step 4: Add re-export, run tests

Add `ErrorRoute` to `pub use detail_types::` in `lib.rs`.

```bash
rtk cargo nextest run -p nebula-error
```

### Step 5: Commit

```bash
rtk git add crates/error/src/detail_types.rs crates/error/src/lib.rs
rtk git commit -m "feat(error): add ErrorRoute detail type for error-edge routing"
```

---

## Task 4: TypeMismatch Detail Type

Addresses **Cluster 5 (Data Passing)** — silent type casts like Flyte#4505 (int→float)

When nebula-validator or typed ports detect a type mismatch between DAG edges, this detail carries the expected vs actual types for diagnostics.

**Files:**
- Modify: `crates/error/src/detail_types.rs`
- Modify: `crates/error/src/lib.rs`
- Test: inline in `detail_types.rs`

### Step 1: Write the failing test

```rust
#[test]
fn type_mismatch_stored_and_retrieved() {
    let mut details = ErrorDetails::new();
    details.insert(TypeMismatch {
        expected: "JsonObject".into(),
        actual: "JsonArray".into(),
        location: Some("edge from http-fetch → parse-response".into()),
    });

    let tm = details.get::<TypeMismatch>().unwrap();
    assert_eq!(tm.expected, "JsonObject");
    assert_eq!(tm.actual, "JsonArray");
    assert!(tm.location.is_some());
}
```

### Step 2: Run to verify failure, then implement

Add to `detail_types.rs`:

```rust
/// Type mismatch between connected DAG nodes.
///
/// Attached when a type validation check detects that an upstream
/// node's output type doesn't match a downstream node's expected
/// input type. This prevents silent casts and data corruption.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, TypeMismatch};
///
/// let mut details = ErrorDetails::new();
/// details.insert(TypeMismatch {
///     expected: "u64".into(),
///     actual: "f64".into(),
///     location: Some("edge: fetch → transform".into()),
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeMismatch {
    /// The expected type name.
    pub expected: String,
    /// The actual type name.
    pub actual: String,
    /// Where in the DAG this mismatch was detected.
    pub location: Option<String>,
}

impl ErrorDetail for TypeMismatch {}
```

### Step 3: Re-export, test, commit

```bash
rtk cargo nextest run -p nebula-error type_mismatch_stored
rtk git add crates/error/src/detail_types.rs crates/error/src/lib.rs
rtk git commit -m "feat(error): add TypeMismatch detail type for DAG edge validation"
```

---

## Task 5: ErrorCategory → HTTP Status Code Mapping

Fills the empty `convert.rs` — needed for the API layer.

**Files:**
- Modify: `crates/error/src/convert.rs`
- Test: inline in `convert.rs`

### Step 1: Write the failing test

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_maps_to_404() {
        assert_eq!(ErrorCategory::NotFound.http_status_code(), 404);
    }

    #[test]
    fn validation_maps_to_400() {
        assert_eq!(ErrorCategory::Validation.http_status_code(), 400);
    }

    #[test]
    fn authentication_maps_to_401() {
        assert_eq!(ErrorCategory::Authentication.http_status_code(), 401);
    }

    #[test]
    fn authorization_maps_to_403() {
        assert_eq!(ErrorCategory::Authorization.http_status_code(), 403);
    }

    #[test]
    fn conflict_maps_to_409() {
        assert_eq!(ErrorCategory::Conflict.http_status_code(), 409);
    }

    #[test]
    fn rate_limit_maps_to_429() {
        assert_eq!(ErrorCategory::RateLimit.http_status_code(), 429);
    }

    #[test]
    fn timeout_maps_to_504() {
        assert_eq!(ErrorCategory::Timeout.http_status_code(), 504);
    }

    #[test]
    fn exhausted_maps_to_429() {
        assert_eq!(ErrorCategory::Exhausted.http_status_code(), 429);
    }

    #[test]
    fn cancelled_maps_to_499() {
        assert_eq!(ErrorCategory::Cancelled.http_status_code(), 499);
    }

    #[test]
    fn internal_maps_to_500() {
        assert_eq!(ErrorCategory::Internal.http_status_code(), 500);
    }

    #[test]
    fn external_maps_to_502() {
        assert_eq!(ErrorCategory::External.http_status_code(), 502);
    }

    #[test]
    fn unsupported_maps_to_501() {
        assert_eq!(ErrorCategory::Unsupported.http_status_code(), 501);
    }

    #[test]
    fn round_trip_from_http_status() {
        for cat in [
            ErrorCategory::NotFound,
            ErrorCategory::Validation,
            ErrorCategory::Authentication,
            ErrorCategory::Authorization,
            ErrorCategory::Internal,
        ] {
            let code = cat.http_status_code();
            let recovered = ErrorCategory::from_http_status(code);
            assert_eq!(recovered, Some(cat));
        }
    }

    #[test]
    fn unknown_status_returns_none() {
        assert_eq!(ErrorCategory::from_http_status(418), None);
    }
}
```

### Step 2: Implement on `ErrorCategory`

Replace the content of `convert.rs`:

```rust
//! Error conversion utilities.
//!
//! Provides bidirectional mapping between [`ErrorCategory`] and HTTP status codes.
//! Additional protocol bridges (gRPC, etc.) will be added behind feature flags.

use crate::ErrorCategory;

impl ErrorCategory {
    /// Maps this category to an HTTP status code.
    ///
    /// The mapping follows standard HTTP semantics:
    ///
    /// | Category | HTTP Status |
    /// |----------|-------------|
    /// | NotFound | 404 |
    /// | Validation | 400 |
    /// | Authentication | 401 |
    /// | Authorization | 403 |
    /// | Conflict | 409 |
    /// | RateLimit | 429 |
    /// | Timeout | 504 |
    /// | Exhausted | 429 |
    /// | Cancelled | 499 |
    /// | Internal | 500 |
    /// | External | 502 |
    /// | Unsupported | 501 |
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert_eq!(ErrorCategory::NotFound.http_status_code(), 404);
    /// assert_eq!(ErrorCategory::Internal.http_status_code(), 500);
    /// ```
    pub const fn http_status_code(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::Validation => 400,
            Self::Authentication => 401,
            Self::Authorization => 403,
            Self::Conflict => 409,
            Self::RateLimit => 429,
            Self::Timeout => 504,
            Self::Exhausted => 429,
            Self::Cancelled => 499,
            Self::Internal => 500,
            Self::External => 502,
            Self::Unsupported => 501,
        }
    }

    /// Attempts to recover an error category from an HTTP status code.
    ///
    /// Returns `None` for status codes that don't map to a known category.
    /// When multiple categories share a status code (e.g. RateLimit and Exhausted
    /// both map to 429), the more common interpretation is returned (RateLimit).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert_eq!(ErrorCategory::from_http_status(404), Some(ErrorCategory::NotFound));
    /// assert_eq!(ErrorCategory::from_http_status(418), None);
    /// ```
    pub const fn from_http_status(status: u16) -> Option<Self> {
        match status {
            400 => Some(Self::Validation),
            401 => Some(Self::Authentication),
            403 => Some(Self::Authorization),
            404 => Some(Self::NotFound),
            409 => Some(Self::Conflict),
            429 => Some(Self::RateLimit),
            499 => Some(Self::Cancelled),
            500 => Some(Self::Internal),
            501 => Some(Self::Unsupported),
            502 => Some(Self::External),
            504 => Some(Self::Timeout),
            _ => None,
        }
    }
}
```

### Step 3: Run tests

```bash
rtk cargo nextest run -p nebula-error -- convert
```

### Step 4: Commit

```bash
rtk git add crates/error/src/convert.rs
rtk git commit -m "feat(error): implement ErrorCategory <-> HTTP status code mapping"
```

---

## Task 6: NebulaError Ergonomic Extensions

DX improvements identified from usage patterns across 21 consuming crates.

**Files:**
- Modify: `crates/error/src/error.rs`
- Modify: `crates/error/src/details.rs`
- Test: inline in both files

### Step 6a: `NebulaError::map_inner` for error type transformation

Needed when converting between crate-specific error types (e.g. `NebulaError<StorageError>` → `NebulaError<EngineError>`).

### Step 1: Write the failing test

In `error.rs` tests:

```rust
#[test]
fn map_inner_transforms_error_type() {
    #[derive(Debug, Clone)]
    struct OtherError(ErrorCategory);

    impl fmt::Display for OtherError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "other({})", self.0)
        }
    }

    impl Classify for OtherError {
        fn category(&self) -> ErrorCategory {
            self.0
        }
        fn code(&self) -> ErrorCode {
            codes::EXTERNAL.clone()
        }
    }

    let original = NebulaError::new(make_error())
        .with_message("test msg")
        .context("ctx1");

    let mapped = original.map_inner(|_inner| OtherError(ErrorCategory::External));

    assert_eq!(mapped.category(), ErrorCategory::External);
    assert_eq!(mapped.to_string(), "test msg");
    assert_eq!(mapped.context_chain().len(), 1);
}
```

### Step 2: Implement `map_inner`

In `error.rs`, add to `impl<E: Classify> NebulaError<E>`:

```rust
/// Transforms the inner error type while preserving all metadata
/// (message, details, context chain, source).
///
/// Useful for converting between crate-specific error types when
/// propagating errors across crate boundaries.
///
/// # Examples
///
/// ```
/// use nebula_error::{Classify, ErrorCategory, ErrorCode, NebulaError, codes};
///
/// # #[derive(Debug)]
/// # struct A;
/// # impl std::fmt::Display for A {
/// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("a") }
/// # }
/// # impl Classify for A {
/// #     fn category(&self) -> ErrorCategory { ErrorCategory::Internal }
/// #     fn code(&self) -> ErrorCode { codes::INTERNAL.clone() }
/// # }
/// # #[derive(Debug)]
/// # struct B;
/// # impl std::fmt::Display for B {
/// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("b") }
/// # }
/// # impl Classify for B {
/// #     fn category(&self) -> ErrorCategory { ErrorCategory::External }
/// #     fn code(&self) -> ErrorCode { codes::EXTERNAL.clone() }
/// # }
/// let err = NebulaError::new(A).with_message("msg").context("ctx");
/// let mapped: NebulaError<B> = err.map_inner(|_| B);
/// assert_eq!(mapped.to_string(), "msg");
/// ```
pub fn map_inner<F: Classify>(self, f: impl FnOnce(E) -> F) -> NebulaError<F> {
    NebulaError {
        inner: f(self.inner),
        message: self.message,
        details: self.details,
        context_chain: self.context_chain,
        source: self.source,
    }
}
```

### Step 3: Run test

```bash
rtk cargo nextest run -p nebula-error map_inner_transforms
```

### Step 6b: `ErrorDetails::remove`

### Step 4: Write the failing test

In `details.rs` tests:

```rust
#[test]
fn remove_returns_value() {
    let mut details = ErrorDetails::new();
    details.insert(Alpha { value: 42 });

    let removed = details.remove::<Alpha>();
    assert_eq!(removed.unwrap().value, 42);
    assert!(!details.has::<Alpha>());
}

#[test]
fn remove_missing_returns_none() {
    let mut details = ErrorDetails::new();
    let removed = details.remove::<Alpha>();
    assert!(removed.is_none());
}
```

### Step 5: Implement `remove`

In `details.rs`, add to `impl ErrorDetails`:

```rust
/// Removes and returns the stored value of type `T`, if present.
pub fn remove<T: ErrorDetail>(&mut self) -> Option<T> {
    self.map
        .remove(&TypeId::of::<T>())
        .and_then(|boxed| boxed.downcast::<T>().ok())
        .map(|boxed| *boxed)
}
```

### Step 6: Run all tests and commit

```bash
rtk cargo nextest run -p nebula-error
rtk git add crates/error/src/error.rs crates/error/src/details.rs
rtk git commit -m "feat(error): add map_inner() and ErrorDetails::remove()"
```

---

## Task 7: Full Workspace Validation

### Step 1: Run full validation

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace
```

### Step 2: Run doctests

```bash
rtk cargo test --workspace --doc
```

### Step 3: Update context file

Verify `.claude/crates/error.md` reflects new types and methods:
- Add `ExecutionContext`, `ErrorRoute`, `TypeMismatch` to detail types
- Add `ErrorClassifier` to traits
- Add `http_status_code()` / `from_http_status()` to ErrorCategory
- Add `map_inner()` to NebulaError
- Add `remove()` to ErrorDetails
- Note RateLimit is now default-retryable

### Step 4: Commit context update

```bash
rtk git add .claude/crates/error.md
rtk git commit -m "docs: update error context file for v2 improvements"
```

---

## Summary of Changes

| Area | What | Why (research cluster) |
|------|------|----------------------|
| `ExecutionContext` detail | node_id, workflow_id, correlation_id, attempt | Cluster 8: traceability |
| `ErrorRoute` detail | suggested_handler, dead_letter | Cluster 10: error routing |
| `TypeMismatch` detail | expected, actual, location | Cluster 5: silent casts |
| `RateLimit` retryable | Added to `is_default_retryable()` | Cluster 6: retry inadequacies |
| `ErrorClassifier` | Predicate-based category filtering | Cluster 6: conditional retry |
| HTTP status mapping | `http_status_code()` / `from_http_status()` | convert.rs was empty |
| `map_inner()` | Error type transformation preserving metadata | DX from 21-crate usage |
| `ErrorDetails::remove()` | Remove detail by type | DX completeness |

**No breaking changes.** All additions are backward-compatible.
