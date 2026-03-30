# nebula-error Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create nebula-error + nebula-error-macros crates providing enterprise error infrastructure (Google error model + AWS SDK pattern adapted to Rust).

**Architecture:** Generic wrapper `NebulaError<E>` with `Classify` trait for error classification. TypeId-keyed `ErrorDetails` for extensible detail types (Google's `Any` pattern). Derive macro `#[derive(Classify)]` for zero-boilerplate adoption.

**Tech Stack:** Rust 1.93, thiserror 2, serde (optional feature), syn/quote/proc-macro2 (macro crate)

**Design doc:** `docs/plans/2026-03-27-nebula-error-design.md`

---

### Task 1: Crate Scaffold

**Files:**
- Create: `crates/error/Cargo.toml`
- Create: `crates/error/src/lib.rs`
- Create: `crates/error-macros/Cargo.toml`
- Create: `crates/error-macros/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create `crates/error/Cargo.toml`**

```toml
[package]
name = "nebula-error"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Enterprise error infrastructure for the Nebula workflow engine"
keywords.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
documentation.workspace = true

[dependencies]
thiserror = { workspace = true }

[dependencies.serde]
workspace = true
optional = true

[dependencies.nebula-error-macros]
path = "../error-macros"
optional = true

[features]
default = []
serde = ["dep:serde"]
derive = ["dep:nebula-error-macros"]

[dev-dependencies]
pretty_assertions = { workspace = true }
serde_json = { workspace = true }
```

**Step 2: Create `crates/error/src/lib.rs`**

```rust
//! # nebula-error
//!
//! Enterprise error infrastructure for the Nebula workflow engine.
//!
//! This crate provides the foundational error primitives used across all Nebula
//! crates — classification traits, a generic error wrapper, and extensible
//! typed details inspired by Google's error model and the AWS SDK.
//!
//! ## Key Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Classify`] | Core trait — category, code, severity, retryability |
//! | [`NebulaError`] | Generic wrapper adding details + context chain |
//! | [`ErrorDetails`] | TypeId-keyed extensible detail storage |
//! | [`ErrorCategory`] | Canonical "what happened" classification |
//! | [`ErrorSeverity`] | Error / Warning / Info severity levels |
//! | [`ErrorCode`] | Machine-readable error code newtype |
//! | [`ErrorCollection`] | Batch/validation error aggregation |
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_error::{Classify, ErrorCategory, ErrorCode, ErrorSeverity};
//!
//! // Implement Classify on your domain error (or use #[derive(Classify)])
//! #[derive(Debug, thiserror::Error)]
//! pub enum MyError {
//!     #[error("connection timed out")]
//!     Timeout,
//! }
//!
//! impl Classify for MyError {
//!     fn category(&self) -> ErrorCategory {
//!         match self {
//!             Self::Timeout => ErrorCategory::Timeout,
//!         }
//!     }
//!     fn error_code(&self) -> ErrorCode {
//!         match self {
//!             Self::Timeout => ErrorCode::new("MY_TIMEOUT"),
//!         }
//!     }
//! }
//! ```

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod category;
mod code;
mod collection;
mod convert;
mod detail_types;
mod details;
mod error;
mod retry;
mod severity;
mod traits;

pub use category::ErrorCategory;
pub use code::{ErrorCode, codes};
pub use collection::{BatchResult, ErrorCollection};
pub use detail_types::{
    BadRequest, DebugInfo, FieldViolation, PreconditionFailure, PreconditionViolation, QuotaInfo,
    ResourceInfo, RetryInfo,
};
pub use details::{ErrorDetail, ErrorDetails};
pub use error::NebulaError;
pub use retry::RetryHint;
pub use severity::ErrorSeverity;
pub use traits::Classify;

/// Convenience result type alias.
pub type Result<T, E> = std::result::Result<T, NebulaError<E>>;

// Re-export derive macro when feature is enabled
#[cfg(feature = "derive")]
pub use nebula_error_macros::Classify;
```

**Step 3: Create `crates/error-macros/Cargo.toml`**

```toml
[package]
name = "nebula-error-macros"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Proc-macros for nebula-error classification"
keywords.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
documentation.workspace = true

[lib]
proc-macro = true

[dependencies]
syn = { version = "2.0", features = ["full", "extra-traits"] }
quote = "1.0"
proc-macro2 = "1.0"
```

**Step 4: Create `crates/error-macros/src/lib.rs` (stub)**

```rust
//! # nebula-error-macros
//!
//! Proc-macros for the [`nebula-error`] crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;
use proc_macro::TokenStream;

/// Derive the `Classify` trait for an error enum.
///
/// See `nebula_error::Classify` for details.
#[proc_macro_derive(Classify, attributes(classify))]
pub fn derive_classify(input: TokenStream) -> TokenStream {
    // Stub — implemented in Task 11
    TokenStream::new()
}
```

**Step 5: Add to workspace `Cargo.toml` members**

Add `"crates/error"` and `"crates/error-macros"` to the `[workspace] members` array.

Also add to `[workspace.dependencies]`:
```toml
nebula-error = { path = "crates/error" }
nebula-error-macros = { path = "crates/error-macros" }
```

**Step 6: Verify scaffold compiles**

Run: `rtk cargo check -p nebula-error -p nebula-error-macros`
Expected: PASS (warnings about unused modules are fine — they're empty stubs)

**Step 7: Commit**

```
feat(error): scaffold nebula-error and nebula-error-macros crates
```

---

### Task 2: ErrorSeverity

**Files:**
- Create: `crates/error/src/severity.rs`

**Step 1: Write tests first**

Add to end of `severity.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_error() {
        assert_eq!(ErrorSeverity::default(), ErrorSeverity::Error);
    }

    #[test]
    fn ordering_error_is_highest() {
        assert!(ErrorSeverity::Error > ErrorSeverity::Warning);
        assert!(ErrorSeverity::Warning > ErrorSeverity::Info);
    }

    #[test]
    fn display_lowercase() {
        assert_eq!(ErrorSeverity::Error.to_string(), "error");
        assert_eq!(ErrorSeverity::Warning.to_string(), "warning");
        assert_eq!(ErrorSeverity::Info.to_string(), "info");
    }

    #[test]
    fn is_helpers() {
        assert!(ErrorSeverity::Error.is_error());
        assert!(!ErrorSeverity::Error.is_warning());
        assert!(ErrorSeverity::Warning.is_warning());
        assert!(ErrorSeverity::Info.is_info());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `rtk cargo test -p nebula-error -- severity`
Expected: FAIL (module is empty)

**Step 3: Implement**

```rust
//! Error severity levels.

use core::fmt;

/// Severity of an error occurrence.
///
/// Used by the framework to decide logging level, alerting, and error
/// presentation. Most errors are [`Error`](ErrorSeverity::Error); use
/// [`Warning`](ErrorSeverity::Warning) for degraded-but-functional states and
/// [`Info`](ErrorSeverity::Info) for notable non-failures.
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorSeverity;
///
/// let severity = ErrorSeverity::Warning;
/// assert!(severity.is_warning());
/// assert!(ErrorSeverity::Error > ErrorSeverity::Warning);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ErrorSeverity {
    /// Notable occurrence, not a failure.
    Info = 0,
    /// Degraded state — operation completed with issues.
    Warning = 1,
    /// Operation failed.
    Error = 2,
}

impl ErrorSeverity {
    /// Returns `true` if this is [`ErrorSeverity::Error`].
    pub const fn is_error(self) -> bool {
        matches!(self, Self::Error)
    }

    /// Returns `true` if this is [`ErrorSeverity::Warning`].
    pub const fn is_warning(self) -> bool {
        matches!(self, Self::Warning)
    }

    /// Returns `true` if this is [`ErrorSeverity::Info`].
    pub const fn is_info(self) -> bool {
        matches!(self, Self::Info)
    }
}

impl Default for ErrorSeverity {
    fn default() -> Self {
        Self::Error
    }
}

impl fmt::Display for ErrorSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => f.write_str("error"),
            Self::Warning => f.write_str("warning"),
            Self::Info => f.write_str("info"),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ErrorSeverity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ErrorSeverity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        match s {
            "error" => Ok(Self::Error),
            "warning" => Ok(Self::Warning),
            "info" => Ok(Self::Info),
            other => Err(serde::de::Error::unknown_variant(other, &["error", "warning", "info"])),
        }
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- severity`
Expected: PASS

**Step 5: Commit**

```
feat(error): add ErrorSeverity with ordering and Display
```

---

### Task 3: ErrorCategory

**Files:**
- Create: `crates/error/src/category.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_snake_case() {
        assert_eq!(ErrorCategory::NotFound.to_string(), "not_found");
        assert_eq!(ErrorCategory::RateLimit.to_string(), "rate_limit");
        assert_eq!(ErrorCategory::Internal.to_string(), "internal");
    }

    #[test]
    fn default_retryable_categories() {
        assert!(ErrorCategory::Timeout.is_default_retryable());
        assert!(ErrorCategory::Exhausted.is_default_retryable());
        assert!(ErrorCategory::External.is_default_retryable());

        assert!(!ErrorCategory::NotFound.is_default_retryable());
        assert!(!ErrorCategory::Validation.is_default_retryable());
        assert!(!ErrorCategory::Internal.is_default_retryable());
        assert!(!ErrorCategory::Cancelled.is_default_retryable());
    }

    #[test]
    fn is_client_error() {
        assert!(ErrorCategory::Validation.is_client_error());
        assert!(ErrorCategory::NotFound.is_client_error());
        assert!(ErrorCategory::Authentication.is_client_error());
        assert!(ErrorCategory::Authorization.is_client_error());
        assert!(ErrorCategory::Conflict.is_client_error());

        assert!(!ErrorCategory::Internal.is_client_error());
        assert!(!ErrorCategory::External.is_client_error());
    }

    #[test]
    fn is_server_error() {
        assert!(ErrorCategory::Internal.is_server_error());
        assert!(ErrorCategory::External.is_server_error());
        assert!(ErrorCategory::Timeout.is_server_error());

        assert!(!ErrorCategory::Validation.is_server_error());
    }
}
```

**Step 2: Run tests to verify failure**

Run: `rtk cargo test -p nebula-error -- category`
Expected: FAIL

**Step 3: Implement**

```rust
//! Canonical error categories.

use core::fmt;

/// High-level classification of what went wrong.
///
/// Inspired by [`google.rpc.Code`](https://grpc.github.io/grpc/core/md_doc_statuscodes.html).
/// Used by the framework for retry decisions, HTTP status mapping, and error routing.
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorCategory;
///
/// let cat = ErrorCategory::Timeout;
/// assert!(cat.is_default_retryable());
/// assert!(cat.is_server_error());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCategory {
    /// Requested entity was not found.
    NotFound,
    /// Invalid input or configuration.
    Validation,
    /// Missing or invalid authentication credentials.
    Authentication,
    /// Caller lacks required permissions.
    Authorization,
    /// State conflict (duplicate, optimistic-lock failure).
    Conflict,
    /// Too many requests — caller should back off.
    RateLimit,
    /// Operation exceeded its deadline.
    Timeout,
    /// A resource is exhausted (pool, memory, quota).
    Exhausted,
    /// Operation cancelled by caller or system.
    Cancelled,
    /// Internal error (bug, invariant violation).
    Internal,
    /// An external dependency is unavailable.
    External,
    /// Requested operation is not supported.
    Unsupported,
}

impl ErrorCategory {
    /// Whether this category is retryable by default.
    ///
    /// Individual errors can override via [`Classify::is_retryable`](crate::Classify::is_retryable).
    pub const fn is_default_retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Exhausted | Self::External)
    }

    /// Whether this category represents a client error (bad input, auth, etc.).
    pub const fn is_client_error(self) -> bool {
        matches!(
            self,
            Self::NotFound
                | Self::Validation
                | Self::Authentication
                | Self::Authorization
                | Self::Conflict
                | Self::Unsupported
        )
    }

    /// Whether this category represents a server/infrastructure error.
    pub const fn is_server_error(self) -> bool {
        matches!(
            self,
            Self::Internal | Self::External | Self::Timeout | Self::Exhausted
        )
    }

    /// Returns the snake_case name of this category.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Validation => "validation",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Conflict => "conflict",
            Self::RateLimit => "rate_limit",
            Self::Timeout => "timeout",
            Self::Exhausted => "exhausted",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
            Self::External => "external",
            Self::Unsupported => "unsupported",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ErrorCategory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ErrorCategory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        match s {
            "not_found" => Ok(Self::NotFound),
            "validation" => Ok(Self::Validation),
            "authentication" => Ok(Self::Authentication),
            "authorization" => Ok(Self::Authorization),
            "conflict" => Ok(Self::Conflict),
            "rate_limit" => Ok(Self::RateLimit),
            "timeout" => Ok(Self::Timeout),
            "exhausted" => Ok(Self::Exhausted),
            "cancelled" => Ok(Self::Cancelled),
            "internal" => Ok(Self::Internal),
            "external" => Ok(Self::External),
            "unsupported" => Ok(Self::Unsupported),
            other => Err(serde::de::Error::unknown_variant(other, &[
                "not_found", "validation", "authentication", "authorization",
                "conflict", "rate_limit", "timeout", "exhausted", "cancelled",
                "internal", "external", "unsupported",
            ])),
        }
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- category`
Expected: PASS

**Step 5: Commit**

```
feat(error): add ErrorCategory with retryable/client/server classification
```

---

### Task 4: ErrorCode + codes module

**Files:**
- Create: `crates/error/src/code.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_code() {
        let code = ErrorCode::new("TIMEOUT");
        assert_eq!(code.as_str(), "TIMEOUT");
    }

    #[test]
    fn custom_code() {
        let code = ErrorCode::custom("STRIPE_CARD_DECLINED");
        assert_eq!(code.as_str(), "STRIPE_CARD_DECLINED");
    }

    #[test]
    fn equality() {
        assert_eq!(ErrorCode::new("FOO"), ErrorCode::custom("FOO"));
    }

    #[test]
    fn display() {
        assert_eq!(codes::TIMEOUT.to_string(), "TIMEOUT");
    }

    #[test]
    fn canonical_codes_exist() {
        // Verify all canonical codes are defined
        let all = [
            &codes::NOT_FOUND, &codes::VALIDATION, &codes::TIMEOUT,
            &codes::RATE_LIMIT, &codes::CANCELLED, &codes::INTERNAL,
            &codes::AUTHENTICATION, &codes::AUTHORIZATION,
            &codes::EXHAUSTED, &codes::CONFLICT, &codes::EXTERNAL,
            &codes::UNSUPPORTED,
        ];
        assert_eq!(all.len(), 12);
        for code in all {
            assert!(!code.as_str().is_empty());
        }
    }
}
```

**Step 2: Run tests — expect FAIL**

Run: `rtk cargo test -p nebula-error -- code`

**Step 3: Implement**

```rust
//! Machine-readable error codes.

use std::borrow::Cow;
use core::fmt;

/// A machine-readable error code.
///
/// Supports both static codes (zero-allocation, `const`-constructible) and
/// dynamic codes from plugin developers at runtime.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorCode, codes};
///
/// // Canonical static code
/// let code = codes::TIMEOUT;
/// assert_eq!(code.as_str(), "TIMEOUT");
///
/// // Plugin-specific runtime code
/// let custom = ErrorCode::custom("STRIPE_CARD_DECLINED");
/// assert_eq!(custom.as_str(), "STRIPE_CARD_DECLINED");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorCode(Cow<'static, str>);

impl ErrorCode {
    /// Creates an error code from a static string.
    pub const fn new(code: &'static str) -> Self {
        Self(Cow::Borrowed(code))
    }

    /// Creates an error code from a runtime string.
    pub fn custom(code: impl Into<String>) -> Self {
        Self(Cow::Owned(code.into()))
    }

    /// Returns the code as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::custom(s))
    }
}

/// Canonical error codes matching [`ErrorCategory`](crate::ErrorCategory) variants.
///
/// Plugin developers should define their own codes with a crate-specific prefix
/// (e.g., `STRIPE_CARD_DECLINED`, `PG_CONNECTION_REFUSED`).
pub mod codes {
    use super::ErrorCode;

    /// Entity not found.
    pub const NOT_FOUND: ErrorCode = ErrorCode::new("NOT_FOUND");
    /// Validation failure.
    pub const VALIDATION: ErrorCode = ErrorCode::new("VALIDATION");
    /// Operation timed out.
    pub const TIMEOUT: ErrorCode = ErrorCode::new("TIMEOUT");
    /// Rate limit exceeded.
    pub const RATE_LIMIT: ErrorCode = ErrorCode::new("RATE_LIMIT");
    /// Operation cancelled.
    pub const CANCELLED: ErrorCode = ErrorCode::new("CANCELLED");
    /// Internal error.
    pub const INTERNAL: ErrorCode = ErrorCode::new("INTERNAL");
    /// Authentication failure.
    pub const AUTHENTICATION: ErrorCode = ErrorCode::new("AUTHENTICATION");
    /// Authorization failure.
    pub const AUTHORIZATION: ErrorCode = ErrorCode::new("AUTHORIZATION");
    /// Resource exhausted.
    pub const EXHAUSTED: ErrorCode = ErrorCode::new("EXHAUSTED");
    /// State conflict.
    pub const CONFLICT: ErrorCode = ErrorCode::new("CONFLICT");
    /// External dependency failure.
    pub const EXTERNAL: ErrorCode = ErrorCode::new("EXTERNAL");
    /// Operation not supported.
    pub const UNSUPPORTED: ErrorCode = ErrorCode::new("UNSUPPORTED");
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- code`
Expected: PASS

**Step 5: Commit**

```
feat(error): add ErrorCode newtype with canonical codes module
```

---

### Task 5: RetryHint

**Files:**
- Create: `crates/error/src/retry.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn after_only() {
        let hint = RetryHint::after(Duration::from_secs(30));
        assert_eq!(hint.after, Some(Duration::from_secs(30)));
        assert_eq!(hint.max_attempts, None);
    }

    #[test]
    fn with_max_attempts() {
        let hint = RetryHint::after(Duration::from_secs(5))
            .with_max_attempts(3);
        assert_eq!(hint.after, Some(Duration::from_secs(5)));
        assert_eq!(hint.max_attempts, Some(3));
    }

    #[test]
    fn max_attempts_only() {
        let hint = RetryHint::max_attempts(5);
        assert_eq!(hint.after, None);
        assert_eq!(hint.max_attempts, Some(5));
    }

    #[test]
    fn display() {
        let hint = RetryHint::after(Duration::from_secs(60)).with_max_attempts(3);
        let s = hint.to_string();
        assert!(s.contains("60s"));
        assert!(s.contains("3"));
    }
}
```

**Step 2: Run tests — expect FAIL**

Run: `rtk cargo test -p nebula-error -- retry`

**Step 3: Implement**

```rust
//! Retry hint metadata.

use core::fmt;
use std::time::Duration;

/// A hint for retry logic — advisory, not prescriptive.
///
/// The resilience layer decides the actual retry strategy; this struct
/// lets domain errors suggest minimum delay or attempt limits.
///
/// # Examples
///
/// ```
/// use nebula_error::RetryHint;
/// use std::time::Duration;
///
/// let hint = RetryHint::after(Duration::from_secs(60))
///     .with_max_attempts(3);
///
/// assert_eq!(hint.after, Some(Duration::from_secs(60)));
/// assert_eq!(hint.max_attempts, Some(3));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryHint {
    /// Minimum delay before the next retry attempt.
    pub after: Option<Duration>,
    /// Maximum total attempts (including the initial try).
    pub max_attempts: Option<u32>,
}

impl RetryHint {
    /// Creates a hint with a minimum retry delay.
    #[must_use]
    pub fn after(delay: Duration) -> Self {
        Self {
            after: Some(delay),
            max_attempts: None,
        }
    }

    /// Creates a hint with only a max-attempts limit.
    #[must_use]
    pub fn max_attempts(n: u32) -> Self {
        Self {
            after: None,
            max_attempts: Some(n),
        }
    }

    /// Sets the maximum number of attempts.
    #[must_use]
    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = Some(n);
        self
    }
}

impl fmt::Display for RetryHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.after, self.max_attempts) {
            (Some(d), Some(n)) => write!(f, "retry after {}s, max {} attempts", d.as_secs(), n),
            (Some(d), None) => write!(f, "retry after {}s", d.as_secs()),
            (None, Some(n)) => write!(f, "max {} attempts", n),
            (None, None) => f.write_str("retry"),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for RetryHint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("RetryHint", 2)?;
        state.serialize_field("after_ms", &self.after.map(|d| d.as_millis() as u64))?;
        state.serialize_field("max_attempts", &self.max_attempts)?;
        state.end()
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- retry`
Expected: PASS

**Step 5: Commit**

```
feat(error): add RetryHint advisory retry metadata
```

---

### Task 6: ErrorDetails + ErrorDetail trait

**Files:**
- Create: `crates/error/src/details.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestDetail {
        value: String,
    }
    impl ErrorDetail for TestDetail {}

    #[derive(Debug)]
    struct OtherDetail(u32);
    impl ErrorDetail for OtherDetail {}

    #[test]
    fn insert_and_get() {
        let mut details = ErrorDetails::new();
        details.insert(TestDetail { value: "hello".into() });
        let got = details.get::<TestDetail>().unwrap();
        assert_eq!(got.value, "hello");
    }

    #[test]
    fn get_missing_returns_none() {
        let details = ErrorDetails::new();
        assert!(details.get::<TestDetail>().is_none());
    }

    #[test]
    fn has_check() {
        let mut details = ErrorDetails::new();
        assert!(!details.has::<TestDetail>());
        details.insert(TestDetail { value: "x".into() });
        assert!(details.has::<TestDetail>());
    }

    #[test]
    fn multiple_types() {
        let mut details = ErrorDetails::new();
        details.insert(TestDetail { value: "a".into() });
        details.insert(OtherDetail(42));

        assert_eq!(details.get::<TestDetail>().unwrap().value, "a");
        assert_eq!(details.get::<OtherDetail>().unwrap().0, 42);
        assert_eq!(details.len(), 2);
    }

    #[test]
    fn insert_overwrites_same_type() {
        let mut details = ErrorDetails::new();
        details.insert(TestDetail { value: "first".into() });
        details.insert(TestDetail { value: "second".into() });
        assert_eq!(details.get::<TestDetail>().unwrap().value, "second");
        assert_eq!(details.len(), 1);
    }

    #[test]
    fn is_empty_and_len() {
        let mut details = ErrorDetails::new();
        assert!(details.is_empty());
        assert_eq!(details.len(), 0);
        details.insert(TestDetail { value: "x".into() });
        assert!(!details.is_empty());
        assert_eq!(details.len(), 1);
    }
}
```

**Step 2: Run tests — expect FAIL**

Run: `rtk cargo test -p nebula-error -- details`

**Step 3: Implement**

```rust
//! Extensible typed error details.
//!
//! Inspired by `google.rpc.Status.details` which uses `google.protobuf.Any`
//! for typed, extensible error metadata. In Rust we use [`TypeId`]-keyed
//! storage for type safety without protobuf overhead.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

/// Marker trait for types that can be stored as error details.
///
/// Implement this on any struct to make it attachable to [`NebulaError`](crate::NebulaError).
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorDetail;
///
/// #[derive(Debug)]
/// struct MyDetail {
///     component: String,
///     request_id: String,
/// }
///
/// impl ErrorDetail for MyDetail {}
/// ```
pub trait ErrorDetail: Any + Send + Sync + fmt::Debug {
    // Marker trait — no methods required.
}

/// Type-safe storage for [`ErrorDetail`] types.
///
/// Each detail type is stored once, keyed by [`TypeId`]. Inserting the same
/// type twice overwrites the previous value.
pub struct ErrorDetails {
    inner: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl ErrorDetails {
    /// Creates an empty detail set.
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Inserts a detail, replacing any existing value of the same type.
    pub fn insert<T: ErrorDetail>(&mut self, detail: T) {
        self.inner.insert(TypeId::of::<T>(), Box::new(detail));
    }

    /// Returns a reference to a detail of the given type, if present.
    pub fn get<T: ErrorDetail>(&self) -> Option<&T> {
        self.inner
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// Returns `true` if a detail of the given type is present.
    pub fn has<T: ErrorDetail>(&self) -> bool {
        self.inner.contains_key(&TypeId::of::<T>())
    }

    /// Returns the number of detail entries.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if no details are stored.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Default for ErrorDetails {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ErrorDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorDetails")
            .field("count", &self.inner.len())
            .finish()
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- details`
Expected: PASS

**Step 5: Commit**

```
feat(error): add ErrorDetails TypeId-keyed detail storage
```

---

### Task 7: Standard detail types

**Files:**
- Create: `crates/error/src/detail_types.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorDetails;
    use std::time::Duration;

    #[test]
    fn retry_info_stored_and_retrieved() {
        let mut details = ErrorDetails::new();
        details.insert(RetryInfo {
            retry_delay: Some(Duration::from_secs(30)),
            max_attempts: Some(3),
        });
        let info = details.get::<RetryInfo>().unwrap();
        assert_eq!(info.retry_delay, Some(Duration::from_secs(30)));
        assert_eq!(info.max_attempts, Some(3));
    }

    #[test]
    fn resource_info() {
        let info = ResourceInfo {
            resource_type: "database".into(),
            resource_name: "pg-main".into(),
            owner: Some("team-platform".into()),
        };
        assert_eq!(info.resource_type, "database");
    }

    #[test]
    fn bad_request_with_violations() {
        let br = BadRequest {
            violations: vec![
                FieldViolation {
                    field: "/name".into(),
                    description: "cannot be empty".into(),
                    code: crate::ErrorCode::new("REQUIRED"),
                },
            ],
        };
        assert_eq!(br.violations.len(), 1);
        assert_eq!(br.violations[0].field, "/name");
    }

    #[test]
    fn multiple_detail_types_coexist() {
        let mut details = ErrorDetails::new();
        details.insert(RetryInfo { retry_delay: Some(Duration::from_secs(5)), max_attempts: None });
        details.insert(ResourceInfo { resource_type: "cache".into(), resource_name: "redis".into(), owner: None });
        details.insert(DebugInfo { detail: "conn reset".into(), stack_entries: vec![] });

        assert!(details.has::<RetryInfo>());
        assert!(details.has::<ResourceInfo>());
        assert!(details.has::<DebugInfo>());
        assert!(!details.has::<QuotaInfo>());
        assert_eq!(details.len(), 3);
    }
}
```

**Step 2: Run tests — expect FAIL**

Run: `rtk cargo test -p nebula-error -- detail_types`

**Step 3: Implement**

```rust
//! Standard error detail types.
//!
//! Inspired by [`google.rpc`](https://github.com/googleapis/googleapis/blob/master/google/rpc/error_details.proto)
//! error detail messages. These are shipped with the crate; plugin developers
//! can create additional detail types by implementing [`ErrorDetail`].

use crate::details::ErrorDetail;
use crate::code::ErrorCode;
use std::borrow::Cow;
use std::time::Duration;

/// Retry delay hint.
///
/// Equivalent to `google.rpc.RetryInfo`.
///
/// # Examples
///
/// ```
/// use nebula_error::RetryInfo;
/// use std::time::Duration;
///
/// let info = RetryInfo {
///     retry_delay: Some(Duration::from_secs(60)),
///     max_attempts: Some(3),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryInfo {
    /// Minimum delay before retrying.
    pub retry_delay: Option<Duration>,
    /// Maximum total attempts (including initial).
    pub max_attempts: Option<u32>,
}

impl ErrorDetail for RetryInfo {}

/// Identifies the resource that caused the error.
///
/// Equivalent to `google.rpc.ResourceInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInfo {
    /// Type of the resource (e.g., `"database"`, `"api"`, `"queue"`).
    pub resource_type: Cow<'static, str>,
    /// Name or identifier of the resource.
    pub resource_name: String,
    /// Optional owner of the resource.
    pub owner: Option<String>,
}

impl ErrorDetail for ResourceInfo {}

/// Validation errors with per-field violations.
///
/// Equivalent to `google.rpc.BadRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadRequest {
    /// List of field-level violations.
    pub violations: Vec<FieldViolation>,
}

impl ErrorDetail for BadRequest {}

/// A single field validation violation.
///
/// Equivalent to `google.rpc.BadRequest.FieldViolation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldViolation {
    /// Path to the field (JSON Pointer RFC 6901, e.g., `"/config/timeout"`).
    pub field: String,
    /// Human-readable description.
    pub description: String,
    /// Machine-readable violation code.
    pub code: ErrorCode,
}

/// Debug context for operators (not end users).
///
/// Equivalent to `google.rpc.DebugInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugInfo {
    /// Detailed debug message.
    pub detail: String,
    /// Stack trace entries.
    pub stack_entries: Vec<String>,
}

impl ErrorDetail for DebugInfo {}

/// Quota or rate-limit failure metadata.
///
/// Equivalent to `google.rpc.QuotaFailure`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaInfo {
    /// Name of the quota metric (e.g., `"requests_per_minute"`).
    pub metric: String,
    /// The enforced quota limit.
    pub limit: u64,
    /// Current usage.
    pub used: u64,
}

impl ErrorDetail for QuotaInfo {}

/// Precondition violations.
///
/// Equivalent to `google.rpc.PreconditionFailure`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreconditionFailure {
    /// List of precondition violations.
    pub violations: Vec<PreconditionViolation>,
}

impl ErrorDetail for PreconditionFailure {}

/// A single precondition violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreconditionViolation {
    /// Type of the precondition (e.g., `"TOS"`, `"STATE"`).
    pub r#type: String,
    /// Subject of the violation.
    pub subject: String,
    /// Human-readable description.
    pub description: String,
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- detail_types`
Expected: PASS

**Step 5: Commit**

```
feat(error): add standard detail types (RetryInfo, ResourceInfo, BadRequest, etc.)
```

---

### Task 8: Classify trait

**Files:**
- Create: `crates/error/src/traits.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCategory, ErrorCode, ErrorSeverity, RetryHint};

    #[derive(Debug)]
    enum TestError {
        Timeout,
        InvalidInput,
        RateLimited,
    }

    impl Classify for TestError {
        fn category(&self) -> ErrorCategory {
            match self {
                Self::Timeout => ErrorCategory::Timeout,
                Self::InvalidInput => ErrorCategory::Validation,
                Self::RateLimited => ErrorCategory::RateLimit,
            }
        }

        fn error_code(&self) -> ErrorCode {
            match self {
                Self::Timeout => ErrorCode::new("TEST_TIMEOUT"),
                Self::InvalidInput => ErrorCode::new("TEST_INVALID"),
                Self::RateLimited => ErrorCode::new("TEST_RATE_LIMIT"),
            }
        }
    }

    #[test]
    fn default_severity_is_error() {
        assert_eq!(TestError::Timeout.severity(), ErrorSeverity::Error);
    }

    #[test]
    fn default_retryable_from_category() {
        assert!(TestError::Timeout.is_retryable());       // Timeout is retryable
        assert!(!TestError::InvalidInput.is_retryable());  // Validation is not
        assert!(!TestError::RateLimited.is_retryable());   // RateLimit not default retryable
    }

    #[test]
    fn default_retry_hint_is_none() {
        assert!(TestError::Timeout.retry_hint().is_none());
    }

    #[test]
    fn custom_overrides() {
        #[derive(Debug)]
        struct CustomError;

        impl Classify for CustomError {
            fn category(&self) -> ErrorCategory { ErrorCategory::RateLimit }
            fn error_code(&self) -> ErrorCode { ErrorCode::new("CUSTOM") }

            fn severity(&self) -> ErrorSeverity { ErrorSeverity::Warning }
            fn is_retryable(&self) -> bool { true }
            fn retry_hint(&self) -> Option<RetryHint> {
                Some(RetryHint::after(std::time::Duration::from_secs(10)))
            }
        }

        assert_eq!(CustomError.severity(), ErrorSeverity::Warning);
        assert!(CustomError.is_retryable());
        assert!(CustomError.retry_hint().is_some());
    }
}
```

**Step 2: Run tests — expect FAIL**

Run: `rtk cargo test -p nebula-error -- traits`

**Step 3: Implement**

```rust
//! The `Classify` trait — core error classification contract.

use crate::category::ErrorCategory;
use crate::code::ErrorCode;
use crate::retry::RetryHint;
use crate::severity::ErrorSeverity;

/// Core error classification trait.
///
/// Every domain error in the Nebula workspace should implement this trait
/// (manually or via `#[derive(Classify)]`). It provides a uniform way for
/// the framework to inspect error category, severity, retryability, and
/// machine-readable codes.
///
/// # Required methods
///
/// Only [`category`](Classify::category) and [`error_code`](Classify::error_code)
/// are required. The remaining methods have sensible defaults.
///
/// # Examples
///
/// ```
/// use nebula_error::{Classify, ErrorCategory, ErrorCode, ErrorSeverity};
///
/// #[derive(Debug)]
/// enum MyError {
///     Timeout,
///     BadInput(String),
/// }
///
/// impl Classify for MyError {
///     fn category(&self) -> ErrorCategory {
///         match self {
///             Self::Timeout => ErrorCategory::Timeout,
///             Self::BadInput(_) => ErrorCategory::Validation,
///         }
///     }
///
///     fn error_code(&self) -> ErrorCode {
///         match self {
///             Self::Timeout => ErrorCode::new("MY_TIMEOUT"),
///             Self::BadInput(_) => ErrorCode::new("MY_BAD_INPUT"),
///         }
///     }
/// }
///
/// assert!(MyError::Timeout.is_retryable());
/// assert!(!MyError::BadInput("x".into()).is_retryable());
/// assert_eq!(MyError::Timeout.severity(), ErrorSeverity::Error);
/// ```
pub trait Classify {
    /// High-level classification of this error.
    fn category(&self) -> ErrorCategory;

    /// Machine-readable error code.
    fn error_code(&self) -> ErrorCode;

    /// Severity level. Defaults to [`ErrorSeverity::Error`].
    fn severity(&self) -> ErrorSeverity {
        ErrorSeverity::Error
    }

    /// Whether this error is worth retrying.
    ///
    /// Default: derived from [`category`](Classify::category) — `Timeout`,
    /// `Exhausted`, and `External` are retryable.
    fn is_retryable(&self) -> bool {
        self.category().is_default_retryable()
    }

    /// Advisory retry hint. Defaults to `None`.
    fn retry_hint(&self) -> Option<RetryHint> {
        None
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- traits`
Expected: PASS

**Step 5: Commit**

```
feat(error): add Classify trait with default retryability from category
```

---

### Task 9: NebulaError<E> wrapper

**Files:**
- Create: `crates/error/src/error.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCategory, ErrorCode, ErrorSeverity, RetryHint};
    use crate::detail_types::{ResourceInfo, RetryInfo};
    use std::time::Duration;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("timed out")]
        Timeout,
        #[error("bad input: {0}")]
        BadInput(String),
    }

    impl Classify for TestError {
        fn category(&self) -> ErrorCategory {
            match self {
                Self::Timeout => ErrorCategory::Timeout,
                Self::BadInput(_) => ErrorCategory::Validation,
            }
        }
        fn error_code(&self) -> ErrorCode {
            match self {
                Self::Timeout => ErrorCode::new("TEST_TIMEOUT"),
                Self::BadInput(_) => ErrorCode::new("TEST_BAD_INPUT"),
            }
        }
    }

    #[test]
    fn new_wraps_inner() {
        let err = NebulaError::new(TestError::Timeout);
        assert_eq!(err.category(), ErrorCategory::Timeout);
        assert_eq!(err.error_code().as_str(), "TEST_TIMEOUT");
        assert!(err.is_retryable());
    }

    #[test]
    fn from_conversion() {
        let err: NebulaError<TestError> = TestError::BadInput("x".into()).into();
        assert_eq!(err.category(), ErrorCategory::Validation);
        assert!(!err.is_retryable());
    }

    #[test]
    fn with_message_overrides_display() {
        let err = NebulaError::new(TestError::Timeout)
            .with_message("custom message");
        assert_eq!(err.to_string(), "custom message");
    }

    #[test]
    fn default_display_uses_inner() {
        let err = NebulaError::new(TestError::BadInput("oops".into()));
        assert_eq!(err.to_string(), "bad input: oops");
    }

    #[test]
    fn context_chain() {
        let err = NebulaError::new(TestError::Timeout)
            .context("while connecting to DB")
            .context("while executing node A");
        let chain = err.context_chain();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], "while connecting to DB");
        assert_eq!(chain[1], "while executing node A");
    }

    #[test]
    fn with_detail() {
        let err = NebulaError::new(TestError::Timeout)
            .with_detail(RetryInfo {
                retry_delay: Some(Duration::from_secs(30)),
                max_attempts: None,
            })
            .with_detail(ResourceInfo {
                resource_type: "db".into(),
                resource_name: "pg-main".into(),
                owner: None,
            });

        assert!(err.detail::<RetryInfo>().is_some());
        assert!(err.detail::<ResourceInfo>().is_some());
        assert_eq!(err.detail::<RetryInfo>().unwrap().retry_delay, Some(Duration::from_secs(30)));
    }

    #[test]
    fn into_inner_recovers_domain_error() {
        let err = NebulaError::new(TestError::Timeout);
        let inner = err.into_inner();
        assert!(matches!(inner, TestError::Timeout));
    }

    #[test]
    fn with_source_chains_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broke");
        let err = NebulaError::new(TestError::Timeout).with_source(io_err);
        assert!(err.source().is_some());
    }

    #[test]
    fn severity_delegates_to_inner() {
        let err = NebulaError::new(TestError::Timeout);
        assert_eq!(err.severity(), ErrorSeverity::Error);
    }
}
```

**Step 2: Run tests — expect FAIL**

Run: `rtk cargo test -p nebula-error -- error`

**Step 3: Implement**

```rust
//! The `NebulaError<E>` generic error wrapper.

use crate::category::ErrorCategory;
use crate::code::ErrorCode;
use crate::details::{ErrorDetail, ErrorDetails};
use crate::retry::RetryHint;
use crate::severity::ErrorSeverity;
use crate::traits::Classify;

use std::borrow::Cow;
use std::error::Error;
use std::fmt;

/// Generic error wrapper that enriches a domain error `E` with
/// infrastructure context.
///
/// The domain error provides classification via [`Classify`]; the wrapper
/// adds a context chain, typed details (Google's `Any` pattern), and an
/// optional source error for chaining.
///
/// # Construction
///
/// ```
/// use nebula_error::{NebulaError, Classify, ErrorCategory, ErrorCode};
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("timed out")]
/// struct Timeout;
///
/// impl Classify for Timeout {
///     fn category(&self) -> ErrorCategory { ErrorCategory::Timeout }
///     fn error_code(&self) -> ErrorCode { ErrorCode::new("TIMEOUT") }
/// }
///
/// let err = NebulaError::new(Timeout)
///     .context("while connecting to DB");
/// assert!(err.is_retryable());
/// ```
pub struct NebulaError<E: Classify> {
    inner: E,
    message: Option<Cow<'static, str>>,
    details: ErrorDetails,
    context: Vec<Cow<'static, str>>,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl<E: Classify> NebulaError<E> {
    // --- Construction ---

    /// Wraps a domain error.
    pub fn new(inner: E) -> Self {
        Self {
            inner,
            message: None,
            details: ErrorDetails::new(),
            context: Vec::new(),
            source: None,
        }
    }

    // --- Builder ---

    /// Overrides the display message.
    #[must_use]
    pub fn with_message(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.message = Some(msg.into());
        self
    }

    /// Sets the source (underlying cause) error.
    #[must_use]
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Attaches a typed detail.
    #[must_use]
    pub fn with_detail<D: ErrorDetail>(mut self, detail: D) -> Self {
        self.details.insert(detail);
        self
    }

    /// Adds a context message to the chain.
    #[must_use]
    pub fn context(mut self, ctx: impl Into<Cow<'static, str>>) -> Self {
        self.context.push(ctx.into());
        self
    }

    // --- Accessors (delegate to Classify) ---

    /// Returns the error category.
    pub fn category(&self) -> ErrorCategory {
        self.inner.category()
    }

    /// Returns the error severity.
    pub fn severity(&self) -> ErrorSeverity {
        self.inner.severity()
    }

    /// Returns the machine-readable error code.
    pub fn error_code(&self) -> ErrorCode {
        self.inner.error_code()
    }

    /// Returns whether this error is retryable.
    pub fn is_retryable(&self) -> bool {
        self.inner.is_retryable()
    }

    /// Returns the advisory retry hint.
    pub fn retry_hint(&self) -> Option<RetryHint> {
        self.inner.retry_hint()
    }

    // --- Domain error access ---

    /// Returns a reference to the wrapped domain error.
    pub fn inner(&self) -> &E {
        &self.inner
    }

    /// Unwraps, returning the domain error.
    pub fn into_inner(self) -> E {
        self.inner
    }

    // --- Details ---

    /// Returns a reference to a detail of the given type.
    pub fn detail<D: ErrorDetail>(&self) -> Option<&D> {
        self.details.get::<D>()
    }

    /// Returns the detail storage.
    pub fn details(&self) -> &ErrorDetails {
        &self.details
    }

    /// Returns mutable access to the detail storage.
    pub fn details_mut(&mut self) -> &mut ErrorDetails {
        &mut self.details
    }

    // --- Context ---

    /// Returns the context chain.
    pub fn context_chain(&self) -> &[Cow<'static, str>] {
        &self.context
    }

    /// Returns the source error, if any.
    pub fn source(&self) -> Option<&(dyn Error + Send + Sync)> {
        self.source.as_deref()
    }
}

impl<E: Classify + fmt::Display> fmt::Display for NebulaError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(msg) = &self.message {
            write!(f, "{msg}")?;
        } else {
            write!(f, "{}", self.inner)?;
        }
        Ok(())
    }
}

impl<E: Classify + fmt::Debug> fmt::Debug for NebulaError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NebulaError")
            .field("inner", &self.inner)
            .field("category", &self.inner.category())
            .field("severity", &self.inner.severity())
            .field("code", &self.inner.error_code())
            .field("retryable", &self.inner.is_retryable())
            .field("details", &self.details)
            .field("context", &self.context)
            .finish()
    }
}

impl<E: Classify + fmt::Debug + fmt::Display> Error for NebulaError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as &(dyn Error + 'static))
    }
}

impl<E: Classify> From<E> for NebulaError<E> {
    fn from(inner: E) -> Self {
        Self::new(inner)
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo test -p nebula-error -- error`
Expected: PASS

**Step 5: Commit**

```
feat(error): add NebulaError<E> generic wrapper with details and context chain
```

---

### Task 10: ErrorCollection + type aliases + convert stubs

**Files:**
- Create: `crates/error/src/collection.rs`
- Create: `crates/error/src/convert.rs`

**Step 1: Write tests first**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCategory, ErrorCode, ErrorSeverity, Classify};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("timeout")]
        Timeout,
        #[error("bad input")]
        BadInput,
    }

    impl Classify for TestError {
        fn category(&self) -> ErrorCategory {
            match self {
                Self::Timeout => ErrorCategory::Timeout,
                Self::BadInput => ErrorCategory::Validation,
            }
        }
        fn error_code(&self) -> ErrorCode {
            match self {
                Self::Timeout => ErrorCode::new("TIMEOUT"),
                Self::BadInput => ErrorCode::new("BAD_INPUT"),
            }
        }
    }

    #[test]
    fn empty_collection() {
        let coll: ErrorCollection<TestError> = ErrorCollection::new();
        assert!(coll.is_empty());
        assert_eq!(coll.len(), 0);
    }

    #[test]
    fn push_and_iterate() {
        let mut coll = ErrorCollection::new();
        coll.push(TestError::Timeout.into());
        coll.push(TestError::BadInput.into());
        assert_eq!(coll.len(), 2);
        let categories: Vec<_> = coll.iter().map(|e| e.category()).collect();
        assert_eq!(categories, vec![ErrorCategory::Timeout, ErrorCategory::Validation]);
    }

    #[test]
    fn any_retryable() {
        let mut coll = ErrorCollection::new();
        coll.push(TestError::BadInput.into());
        assert!(!coll.any_retryable());
        coll.push(TestError::Timeout.into());
        assert!(coll.any_retryable());
    }

    #[test]
    fn max_severity() {
        let coll: ErrorCollection<TestError> = ErrorCollection::new();
        assert_eq!(coll.max_severity(), ErrorSeverity::Info); // empty = lowest

        let mut coll = ErrorCollection::new();
        coll.push(TestError::Timeout.into());
        assert_eq!(coll.max_severity(), ErrorSeverity::Error);
    }

    #[test]
    fn uniform_category_same() {
        let mut coll = ErrorCollection::new();
        coll.push(TestError::Timeout.into());
        coll.push(TestError::Timeout.into());
        assert_eq!(coll.uniform_category(), Some(ErrorCategory::Timeout));
    }

    #[test]
    fn uniform_category_mixed() {
        let mut coll = ErrorCollection::new();
        coll.push(TestError::Timeout.into());
        coll.push(TestError::BadInput.into());
        assert_eq!(coll.uniform_category(), None);
    }

    #[test]
    fn into_iterator() {
        let mut coll = ErrorCollection::new();
        coll.push(TestError::Timeout.into());
        let errors: Vec<_> = coll.into_iter().collect();
        assert_eq!(errors.len(), 1);
    }
}
```

**Step 2: Run tests — expect FAIL**

Run: `rtk cargo test -p nebula-error -- collection`

**Step 3: Implement collection.rs**

```rust
//! Error aggregation for batch operations and validation.

use crate::category::ErrorCategory;
use crate::error::NebulaError;
use crate::severity::ErrorSeverity;
use crate::traits::Classify;

/// A collection of errors for batch/validation scenarios.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorCollection, NebulaError, Classify, ErrorCategory, ErrorCode};
///
/// #[derive(Debug, thiserror::Error)]
/// #[error("required")]
/// struct Required;
///
/// impl Classify for Required {
///     fn category(&self) -> ErrorCategory { ErrorCategory::Validation }
///     fn error_code(&self) -> ErrorCode { ErrorCode::new("REQUIRED") }
/// }
///
/// let mut errors = ErrorCollection::new();
/// errors.push(Required.into());
/// assert!(!errors.is_empty());
/// ```
#[derive(Debug)]
pub struct ErrorCollection<E: Classify> {
    errors: Vec<NebulaError<E>>,
}

impl<E: Classify> ErrorCollection<E> {
    /// Creates an empty collection.
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Adds an error to the collection.
    pub fn push(&mut self, error: NebulaError<E>) {
        self.errors.push(error);
    }

    /// Returns `true` if the collection has no errors.
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns the number of errors.
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Iterates over the errors.
    pub fn iter(&self) -> impl Iterator<Item = &NebulaError<E>> {
        self.errors.iter()
    }

    /// Returns `true` if any error in the collection is retryable.
    pub fn any_retryable(&self) -> bool {
        self.errors.iter().any(|e| e.is_retryable())
    }

    /// Returns the highest severity in the collection.
    ///
    /// Returns [`ErrorSeverity::Info`] if the collection is empty.
    pub fn max_severity(&self) -> ErrorSeverity {
        self.errors
            .iter()
            .map(|e| e.severity())
            .max()
            .unwrap_or(ErrorSeverity::Info)
    }

    /// Returns the common category if all errors share one, otherwise `None`.
    pub fn uniform_category(&self) -> Option<ErrorCategory> {
        let mut iter = self.errors.iter();
        let first = iter.next()?.category();
        if iter.all(|e| e.category() == first) {
            Some(first)
        } else {
            None
        }
    }
}

impl<E: Classify> Default for ErrorCollection<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Classify> IntoIterator for ErrorCollection<E> {
    type Item = NebulaError<E>;
    type IntoIter = std::vec::IntoIter<NebulaError<E>>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.into_iter()
    }
}

impl<'a, E: Classify> IntoIterator for &'a ErrorCollection<E> {
    type Item = &'a NebulaError<E>;
    type IntoIter = std::slice::Iter<'a, NebulaError<E>>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.iter()
    }
}

impl<E: Classify> FromIterator<NebulaError<E>> for ErrorCollection<E> {
    fn from_iter<I: IntoIterator<Item = NebulaError<E>>>(iter: I) -> Self {
        Self {
            errors: iter.into_iter().collect(),
        }
    }
}

/// Result alias for batch operations that may produce multiple errors.
pub type BatchResult<T, E> = std::result::Result<T, ErrorCollection<E>>;
```

**Step 4: Create convert.rs stub**

```rust
//! Conversion bridges between nebula-error and external error types.
//!
//! Future home of `From` implementations connecting `NebulaError` to
//! framework-level types (e.g., HTTP responses, gRPC status codes).
```

**Step 5: Run tests**

Run: `rtk cargo test -p nebula-error -- collection`
Expected: PASS

**Step 6: Run full crate tests**

Run: `rtk cargo test -p nebula-error`
Expected: ALL PASS

**Step 7: Commit**

```
feat(error): add ErrorCollection for batch/validation aggregation
```

---

### Task 11: Derive macro — parse #[classify(...)] attributes

**Files:**
- Modify: `crates/error-macros/src/lib.rs`

**Step 1: Write integration test first**

Create `crates/error/tests/derive.rs`:

```rust
//! Integration tests for #[derive(Classify)].

use nebula_error::{Classify, ErrorCategory, ErrorCode, ErrorSeverity};
use std::time::Duration;

#[derive(Debug, thiserror::Error, nebula_error_macros::Classify)]
enum SimpleError {
    #[classify(category = "timeout", code = "SIMPLE_TIMEOUT")]
    #[error("timed out")]
    Timeout,

    #[classify(category = "validation", code = "SIMPLE_INVALID")]
    #[error("invalid")]
    Invalid,
}

#[test]
fn simple_category() {
    assert_eq!(SimpleError::Timeout.category(), ErrorCategory::Timeout);
    assert_eq!(SimpleError::Invalid.category(), ErrorCategory::Validation);
}

#[test]
fn simple_error_code() {
    assert_eq!(SimpleError::Timeout.error_code().as_str(), "SIMPLE_TIMEOUT");
    assert_eq!(SimpleError::Invalid.error_code().as_str(), "SIMPLE_INVALID");
}

#[test]
fn simple_default_severity() {
    assert_eq!(SimpleError::Timeout.severity(), ErrorSeverity::Error);
}

#[test]
fn simple_default_retryable() {
    assert!(SimpleError::Timeout.is_retryable());
    assert!(!SimpleError::Invalid.is_retryable());
}

#[derive(Debug, thiserror::Error, nebula_error_macros::Classify)]
enum FullError {
    #[classify(category = "timeout", code = "FULL_TIMEOUT")]
    #[error("timeout")]
    Timeout,

    #[classify(category = "validation", code = "FULL_WARN", severity = "warning")]
    #[error("warning")]
    SoftWarning,

    #[classify(category = "rate_limit", code = "FULL_RATE", retry_after_secs = 60)]
    #[error("rate limited")]
    RateLimited,

    #[classify(category = "external", code = "FULL_EXT", retryable = false)]
    #[error("external")]
    ExternalNonRetryable,
}

#[test]
fn full_severity_override() {
    assert_eq!(FullError::SoftWarning.severity(), ErrorSeverity::Warning);
    assert_eq!(FullError::Timeout.severity(), ErrorSeverity::Error);
}

#[test]
fn full_retryable_override() {
    assert!(!FullError::ExternalNonRetryable.is_retryable()); // overridden to false
    assert!(FullError::Timeout.is_retryable()); // default from category
}

#[test]
fn full_retry_hint() {
    let hint = FullError::RateLimited.retry_hint().unwrap();
    assert_eq!(hint.after, Some(Duration::from_secs(60)));
    assert!(FullError::Timeout.retry_hint().is_none());
}

#[derive(Debug, thiserror::Error, nebula_error_macros::Classify)]
enum WithFields {
    #[classify(category = "external", code = "API_ERR")]
    #[error("API error {status}: {body}")]
    ApiError { status: u16, body: String },

    #[classify(category = "timeout", code = "CONN_TIMEOUT")]
    #[error("connection timeout after {0:?}")]
    ConnTimeout(Duration),
}

#[test]
fn variants_with_fields() {
    let err = WithFields::ApiError { status: 429, body: "too many".into() };
    assert_eq!(err.category(), ErrorCategory::External);
    assert_eq!(err.error_code().as_str(), "API_ERR");
}
```

**Step 2: Run test to verify it fails**

Run: `rtk cargo test -p nebula-error --test derive`
Expected: FAIL (derive macro is stub returning empty TokenStream)

**Step 3: Implement the derive macro**

Replace `crates/error-macros/src/lib.rs` with full implementation:

```rust
//! # nebula-error-macros
//!
//! Proc-macros for the [`nebula-error`] crate.
//!
//! Provides `#[derive(Classify)]` to auto-implement the `Classify` trait
//! from `#[classify(...)]` attributes on enum variants.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Variant, parse_macro_input};

/// Parsed classification attributes from a single enum variant.
struct VariantClassification {
    ident: Ident,
    fields: Fields,
    category: String,
    code: String,
    severity: Option<String>,
    retryable: Option<bool>,
    retry_after_secs: Option<u64>,
}

/// Derive the `Classify` trait for an error enum.
///
/// Each variant must have a `#[classify(...)]` attribute with at least
/// `category` and `code`. Optional: `severity`, `retryable`, `retry_after_secs`.
///
/// # Example
///
/// ```ignore
/// #[derive(Debug, thiserror::Error, Classify)]
/// enum MyError {
///     #[classify(category = "timeout", code = "MY_TIMEOUT")]
///     #[error("timed out")]
///     Timeout,
///
///     #[classify(category = "validation", code = "MY_INVALID", severity = "warning")]
///     #[error("invalid input")]
///     Invalid,
/// }
/// ```
#[proc_macro_derive(Classify, attributes(classify))]
pub fn derive_classify(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_classify_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn derive_classify_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let variants = match &input.data {
        Data::Enum(data) => &data.variants,
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Classify can only be derived for enums",
            ));
        }
    };

    let classifications: Vec<VariantClassification> = variants
        .iter()
        .map(parse_variant)
        .collect::<syn::Result<Vec<_>>>()?;

    let category_arms = classifications.iter().map(|c| {
        let pat = build_pattern(name, c);
        let cat = category_ident(&c.category);
        quote! { #pat => ::nebula_error::ErrorCategory::#cat }
    });

    let code_arms = classifications.iter().map(|c| {
        let pat = build_pattern(name, c);
        let code_str = &c.code;
        quote! { #pat => ::nebula_error::ErrorCode::new(#code_str) }
    });

    // Severity: only generate match arm if at least one variant overrides
    let has_severity_override = classifications.iter().any(|c| c.severity.is_some());
    let severity_method = if has_severity_override {
        let arms = classifications.iter().map(|c| {
            let pat = build_pattern(name, c);
            match &c.severity {
                Some(s) => {
                    let sev = severity_ident(s);
                    quote! { #pat => ::nebula_error::ErrorSeverity::#sev }
                }
                None => quote! { #pat => ::nebula_error::ErrorSeverity::Error },
            }
        });
        quote! {
            fn severity(&self) -> ::nebula_error::ErrorSeverity {
                match self { #(#arms),* }
            }
        }
    } else {
        quote! {}
    };

    // Retryable: only generate if at least one variant overrides
    let has_retryable_override = classifications.iter().any(|c| c.retryable.is_some());
    let retryable_method = if has_retryable_override {
        let arms = classifications.iter().map(|c| {
            let pat = build_pattern(name, c);
            match c.retryable {
                Some(val) => quote! { #pat => #val },
                None => quote! { #pat => self.category().is_default_retryable() },
            }
        });
        quote! {
            fn is_retryable(&self) -> bool {
                match self { #(#arms),* }
            }
        }
    } else {
        quote! {}
    };

    // RetryHint: only generate if at least one variant has retry_after_secs
    let has_retry_hint = classifications.iter().any(|c| c.retry_after_secs.is_some());
    let retry_hint_method = if has_retry_hint {
        let arms = classifications.iter().map(|c| {
            let pat = build_pattern(name, c);
            match c.retry_after_secs {
                Some(secs) => quote! {
                    #pat => ::core::option::Option::Some(
                        ::nebula_error::RetryHint::after(::core::time::Duration::from_secs(#secs))
                    )
                },
                None => quote! { #pat => ::core::option::Option::None },
            }
        });
        quote! {
            fn retry_hint(&self) -> ::core::option::Option<::nebula_error::RetryHint> {
                match self { #(#arms),* }
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl #impl_generics ::nebula_error::Classify for #name #ty_generics #where_clause {
            fn category(&self) -> ::nebula_error::ErrorCategory {
                match self { #(#category_arms),* }
            }

            fn error_code(&self) -> ::nebula_error::ErrorCode {
                match self { #(#code_arms),* }
            }

            #severity_method
            #retryable_method
            #retry_hint_method
        }
    })
}

fn parse_variant(variant: &Variant) -> syn::Result<VariantClassification> {
    let attr = variant
        .attrs
        .iter()
        .find(|a| a.path().is_ident("classify"))
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &variant.ident,
                "missing #[classify(...)] attribute on variant",
            )
        })?;

    let mut category = None;
    let mut code = None;
    let mut severity = None;
    let mut retryable = None;
    let mut retry_after_secs = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("category") {
            let value = meta.value()?;
            let lit: syn::LitStr = value.parse()?;
            category = Some(lit.value());
        } else if meta.path.is_ident("code") {
            let value = meta.value()?;
            let lit: syn::LitStr = value.parse()?;
            code = Some(lit.value());
        } else if meta.path.is_ident("severity") {
            let value = meta.value()?;
            let lit: syn::LitStr = value.parse()?;
            severity = Some(lit.value());
        } else if meta.path.is_ident("retryable") {
            let value = meta.value()?;
            let lit: syn::LitBool = value.parse()?;
            retryable = Some(lit.value());
        } else if meta.path.is_ident("retry_after_secs") {
            let value = meta.value()?;
            let lit: syn::LitInt = value.parse()?;
            retry_after_secs = Some(lit.base10_parse::<u64>()?);
        } else {
            return Err(meta.error("unknown classify attribute"));
        }
        Ok(())
    })?;

    let category = category.ok_or_else(|| {
        syn::Error::new_spanned(attr, "missing `category` in #[classify(...)]")
    })?;
    let code = code.ok_or_else(|| {
        syn::Error::new_spanned(attr, "missing `code` in #[classify(...)]")
    })?;

    Ok(VariantClassification {
        ident: variant.ident.clone(),
        fields: variant.fields.clone(),
        category,
        code,
        severity,
        retryable,
        retry_after_secs,
    })
}

fn build_pattern(enum_name: &Ident, c: &VariantClassification) -> TokenStream2 {
    let variant = &c.ident;
    match &c.fields {
        Fields::Unit => quote! { #enum_name::#variant },
        Fields::Unnamed(_) => quote! { #enum_name::#variant(..) },
        Fields::Named(_) => quote! { #enum_name::#variant { .. } },
    }
}

fn category_ident(s: &str) -> proc_macro2::Ident {
    let pascal = match s {
        "not_found" => "NotFound",
        "validation" => "Validation",
        "authentication" => "Authentication",
        "authorization" => "Authorization",
        "conflict" => "Conflict",
        "rate_limit" => "RateLimit",
        "timeout" => "Timeout",
        "exhausted" => "Exhausted",
        "cancelled" => "Cancelled",
        "internal" => "Internal",
        "external" => "External",
        "unsupported" => "Unsupported",
        other => panic!("unknown category: {other}"),
    };
    proc_macro2::Ident::new(pascal, proc_macro2::Span::call_site())
}

fn severity_ident(s: &str) -> proc_macro2::Ident {
    let pascal = match s {
        "error" => "Error",
        "warning" => "Warning",
        "info" => "Info",
        other => panic!("unknown severity: {other}"),
    };
    proc_macro2::Ident::new(pascal, proc_macro2::Span::call_site())
}
```

**Step 4: Update Cargo.toml to enable derive in integration tests**

In `crates/error/Cargo.toml`, add to `[dev-dependencies]`:

```toml
nebula-error-macros = { path = "../error-macros" }
```

**Step 5: Run integration tests**

Run: `rtk cargo test -p nebula-error --test derive`
Expected: ALL PASS

**Step 6: Run full test suite**

Run: `rtk cargo test -p nebula-error -p nebula-error-macros`
Expected: ALL PASS

**Step 7: Commit**

```
feat(error): implement #[derive(Classify)] proc-macro
```

---

### Task 12: Serde feature tests + doc tests

**Files:**
- Create: `crates/error/tests/serde.rs`

**Step 1: Write serde integration tests**

```rust
//! Integration tests for serde serialization.
#![cfg(feature = "serde")]

use nebula_error::{ErrorSeverity, ErrorCategory, ErrorCode};

#[test]
fn severity_roundtrip() {
    let json = serde_json::to_string(&ErrorSeverity::Warning).unwrap();
    assert_eq!(json, "\"warning\"");
    let back: ErrorSeverity = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ErrorSeverity::Warning);
}

#[test]
fn category_roundtrip() {
    let json = serde_json::to_string(&ErrorCategory::RateLimit).unwrap();
    assert_eq!(json, "\"rate_limit\"");
    let back: ErrorCategory = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ErrorCategory::RateLimit);
}

#[test]
fn error_code_roundtrip() {
    let code = ErrorCode::new("MY_CODE");
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, "\"MY_CODE\"");
    let back: ErrorCode = serde_json::from_str(&json).unwrap();
    assert_eq!(back.as_str(), "MY_CODE");
}
```

**Step 2: Run with serde feature**

Run: `rtk cargo test -p nebula-error --features serde --test serde`
Expected: PASS

**Step 3: Run doc tests**

Run: `rtk cargo test -p nebula-error --doc`
Expected: PASS

**Step 4: Run clippy**

Run: `rtk cargo clippy -p nebula-error -p nebula-error-macros -- -D warnings`
Expected: PASS (zero warnings)

**Step 5: Run full workspace check to verify no breakage**

Run: `rtk cargo check --workspace`
Expected: PASS

**Step 6: Commit**

```
test(error): add serde roundtrip tests and verify doc tests
```

---

### Task 13: Update context files and finalize

**Files:**
- Modify: `Cargo.toml` (verify workspace deps entry)
- Create: `.claude/crates/error.md`
- Modify: `.claude/active-work.md`

**Step 1: Create `.claude/crates/error.md`**

```markdown
# nebula-error

Enterprise error infrastructure. Google error model (Status + typed details) adapted to Rust with AWS SDK wrapper pattern.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `Classify` trait: 2 required (`category`, `error_code`), 3 optional with defaults
- `is_retryable()` default from `ErrorCategory`: Timeout, Exhausted, External = retryable
- `ErrorDetails` keyed by TypeId — one value per type, insert overwrites
- `ErrorCode` uses `Cow<'static, str>` — static for canonical, owned for plugin runtime codes
- `ErrorSeverity` ordering: Info < Warning < Error (derives Ord)
- `NebulaError<E>` requires `E: Classify` — classification delegated to domain error
- Serde behind feature flag — not forced on all consumers
- Derive macro behind `derive` feature flag

## Traps

- `ErrorCategory` and `ErrorSeverity` are `#[non_exhaustive]` — match arms need wildcard
- `RetryHint` is advisory — resilience layer may ignore it
- `ErrorDetails::insert` overwrites same-type entry silently (no merge)
- Derive macro panics at compile time for unknown category/severity strings

## Relations

- Depends on: thiserror (required), serde (optional), nebula-error-macros (optional)
- Depended on by: (future — all crates during Phase 3 migration)

<!-- reviewed: 2026-03-27 -->
```

**Step 2: Update `.claude/active-work.md`**

Add nebula-error to "Recently Completed" section.

**Step 3: Final validation**

Run: `rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace`
Expected: ALL PASS

**Step 4: Commit**

```
docs(error): add crate context file and update active-work
```

---

## Task Summary

| Task | Description | Estimated effort |
|------|-------------|-----------------|
| 1 | Crate scaffold (Cargo.toml, lib.rs, workspace) | Small |
| 2 | ErrorSeverity | Small |
| 3 | ErrorCategory | Small |
| 4 | ErrorCode + codes module | Small |
| 5 | RetryHint | Small |
| 6 | ErrorDetails + ErrorDetail trait | Small |
| 7 | Standard detail types | Small |
| 8 | Classify trait | Small |
| 9 | NebulaError\<E\> wrapper | Medium |
| 10 | ErrorCollection + type aliases | Small |
| 11 | Derive macro implementation | Medium |
| 12 | Serde tests + doc tests + clippy | Small |
| 13 | Context files + finalize | Small |
