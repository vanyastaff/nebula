//! Unified error types for the resource subsystem.
//!
//! Every resource error carries an [`ErrorKind`] (what happened) and an
//! [`ErrorScope`] (resource-wide vs. target-specific). The framework uses
//! `ErrorKind` to decide whether to retry, back off, or propagate.

use std::{fmt, time::Duration};

use nebula_core::ResourceKey;

/// How the framework should handle this error.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Network blip, timeout — retry with backoff.
    Transient,
    /// Auth failure, invalid config — never retry.
    Permanent,
    /// Rate limit, quota depleted — retry after cooldown.
    Exhausted {
        /// Optional hint for how long the caller should wait before retrying.
        retry_after: Option<Duration>,
    },
    /// Pool/semaphore full — caller decides.
    Backpressure,
    /// Resource key not in registry.
    NotFound,
    /// `CancellationToken` fired.
    Cancelled,
}

/// Whether the error is resource-wide or target-specific.
///
/// Currently a single-variant `#[non_exhaustive]` enum: only [`Resource`]
/// (the default) is constructed by any production code path. Older drafts
/// included a `Target { id: String }` variant for per-target isolation
/// failures (#391); it was removed at register R-051 resolution since no
/// consumer ever wired it. New variants land here when an engine surface
/// genuinely needs them.
///
/// [`Resource`]: ErrorScope::Resource
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ErrorScope {
    /// The resource itself might be broken.
    #[default]
    Resource,
}

/// Unified resource error.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    scope: ErrorScope,
    message: String,
    resource_key: Option<ResourceKey>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    /// Creates a new error with the given kind and message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            scope: ErrorScope::default(),
            message: message.into(),
            resource_key: None,
            source: None,
        }
    }

    /// Returns the error kind.
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Returns the error scope.
    pub fn scope(&self) -> &ErrorScope {
        &self.scope
    }

    /// Returns the resource key, if set.
    pub fn resource_key(&self) -> Option<&ResourceKey> {
        self.resource_key.as_ref()
    }

    /// Returns `true` if the error is retryable.
    ///
    /// `Transient`, `Exhausted`, and `Backpressure` are retryable because they
    /// represent transient conditions that resolve with time or backoff.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Transient | ErrorKind::Exhausted { .. } | ErrorKind::Backpressure
        )
    }

    /// Returns the retry-after hint, if available.
    ///
    /// - `Exhausted` errors carry an explicit `retry_after` from the upstream.
    /// - `Backpressure` errors return a default 50ms hint (pool slots free up quickly).
    pub fn retry_after(&self) -> Option<Duration> {
        match &self.kind {
            ErrorKind::Exhausted { retry_after } => *retry_after,
            ErrorKind::Backpressure => Some(Duration::from_millis(50)),
            _ => None,
        }
    }

    /// Attaches a resource key to this error.
    pub fn with_resource_key(mut self, key: ResourceKey) -> Self {
        self.resource_key = Some(key);
        self
    }

    /// Attaches a source error.
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Sets the error scope.
    pub fn with_scope(mut self, scope: ErrorScope) -> Self {
        self.scope = scope;
        self
    }

    // --- Convenience constructors ---

    /// Creates a transient (retryable) error.
    pub fn transient(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Transient, message)
    }

    /// Creates a permanent (non-retryable) error.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Permanent, message)
    }

    /// Creates an exhausted (rate-limited) error.
    pub fn exhausted(message: impl Into<String>, retry_after: Option<Duration>) -> Self {
        Self::new(ErrorKind::Exhausted { retry_after }, message)
    }

    /// Creates a not-found error for a missing resource key.
    pub fn not_found(key: &ResourceKey) -> Self {
        Self::new(ErrorKind::NotFound, format!("resource not found: {key}"))
            .with_resource_key(key.clone())
    }

    /// Creates a cancelled error.
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "operation cancelled")
    }

    /// Creates a backpressure error.
    pub fn backpressure(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Backpressure, message)
    }

    /// A credential-bearing resource was registered without a `credential_id`.
    ///
    /// Resources whose `Credential` associated type is not `NoCredential`
    /// must be registered with a `credential_id` so the engine can project
    /// scheme material at acquire time and dispatch rotation hooks.
    pub fn missing_credential_id(key: ResourceKey) -> Self {
        Self::permanent(format!(
            "{key}: credential-bearing Resource (Credential != NoCredential) requires a credential_id at register time"
        ))
        .with_resource_key(key)
    }

    /// A dispatcher failed to downcast `&(dyn Any)` to the expected
    /// `<R::Credential as Credential>::Scheme`.
    ///
    /// Indicates a dispatcher bug — the engine passed a scheme of the wrong
    /// type to a typed rotation hook. Returned as `Permanent` so it
    /// surfaces immediately rather than triggering retry loops.
    pub fn scheme_type_mismatch<R: crate::resource::Resource>() -> Self {
        let key = R::key();
        Self::permanent(format!(
            "{key}: scheme type mismatch — dispatcher expected <{} as Credential>::Scheme",
            std::any::type_name::<R::Credential>()
        ))
        .with_resource_key(key)
    }
}

// ---------------------------------------------------------------------------
// Rotation outcomes (П2)
// ---------------------------------------------------------------------------

/// Outcome of a single resource's `on_credential_refresh` invocation.
///
/// Produced by the rotation dispatcher (one per resource per refresh cycle).
/// Aggregated into [`RotationOutcome`] for event payloads and metrics.
///
/// Not `Clone` because [`Error`] carries a non-cloneable source chain;
/// outcomes are consumed once when folded into the aggregate.
#[derive(Debug)]
pub enum RefreshOutcome {
    /// Resource successfully applied the new scheme.
    Ok,
    /// Resource returned an error from `on_credential_refresh`.
    Failed(Error),
    /// Per-resource timeout budget exceeded.
    TimedOut {
        /// The per-resource budget that was exceeded.
        budget: Duration,
    },
}

/// Outcome of a single resource's `on_credential_revoke` invocation.
///
/// Produced by the rotation dispatcher (one per resource per revoke cycle).
/// Aggregated into [`RotationOutcome`] for event payloads and metrics.
///
/// Not `Clone` because [`Error`] carries a non-cloneable source chain;
/// outcomes are consumed once when folded into the aggregate.
#[derive(Debug)]
pub enum RevokeOutcome {
    /// Resource successfully tore down credential-bound state.
    Ok,
    /// Resource returned an error from `on_credential_revoke`.
    Failed(Error),
    /// Per-resource timeout budget exceeded.
    TimedOut {
        /// The per-resource budget that was exceeded.
        budget: Duration,
    },
}

/// Aggregate counts derived from a rotation cycle's per-resource outcomes.
///
/// Constructed at event-emission time from `Vec<(ResourceKey, RefreshOutcome)>`
/// or its revoke counterpart. Used in `ResourceEvent::CredentialRefreshed` /
/// `CredentialRevoked` payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RotationOutcome {
    /// Number of resources that applied the rotation successfully.
    pub ok: usize,
    /// Number of resources that returned an error from the hook.
    pub failed: usize,
    /// Number of resources whose hook exceeded the per-resource budget.
    pub timed_out: usize,
}

impl RotationOutcome {
    /// Total resources affected by the rotation cycle.
    pub fn total(&self) -> usize {
        self.ok + self.failed + self.timed_out
    }

    /// True if any resource did not complete the hook successfully.
    pub fn has_partial_failure(&self) -> bool {
        self.failed + self.timed_out > 0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref key) = self.resource_key {
            write!(f, "[{key}] ")?;
        }
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl nebula_error::Classify for Error {
    fn category(&self) -> nebula_error::ErrorCategory {
        match &self.kind {
            ErrorKind::Transient => nebula_error::ErrorCategory::External,
            ErrorKind::Permanent => nebula_error::ErrorCategory::Internal,
            ErrorKind::Exhausted { .. } => nebula_error::ErrorCategory::Exhausted,
            ErrorKind::Backpressure => nebula_error::ErrorCategory::RateLimit,
            ErrorKind::NotFound => nebula_error::ErrorCategory::NotFound,
            ErrorKind::Cancelled => nebula_error::ErrorCategory::Cancelled,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match &self.kind {
            ErrorKind::Transient => "RESOURCE:TRANSIENT",
            ErrorKind::Permanent => "RESOURCE:PERMANENT",
            ErrorKind::Exhausted { .. } => "RESOURCE:EXHAUSTED",
            ErrorKind::Backpressure => "RESOURCE:BACKPRESSURE",
            ErrorKind::NotFound => "RESOURCE:NOT_FOUND",
            ErrorKind::Cancelled => "RESOURCE:CANCELLED",
        })
    }

    fn is_retryable(&self) -> bool {
        self.is_retryable()
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        self.retry_after().map(nebula_error::RetryHint::after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_is_retryable() {
        let err = Error::transient("timeout");
        assert!(err.is_retryable());
        assert_eq!(*err.kind(), ErrorKind::Transient);
    }

    #[test]
    fn permanent_is_not_retryable() {
        let err = Error::permanent("bad config");
        assert!(!err.is_retryable());
        assert_eq!(*err.kind(), ErrorKind::Permanent);
    }

    #[test]
    fn exhausted_carries_retry_after() {
        let err = Error::exhausted("rate limited", Some(Duration::from_secs(30)));
        assert!(err.is_retryable());
        assert_eq!(err.retry_after(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn exhausted_without_retry_after() {
        let err = Error::exhausted("quota depleted", None);
        assert!(err.is_retryable());
        assert_eq!(err.retry_after(), None);
    }

    #[test]
    fn not_found_carries_resource_key() {
        let key = nebula_core::resource_key!("postgres");
        let err = Error::not_found(&key);
        assert_eq!(*err.kind(), ErrorKind::NotFound);
        assert_eq!(err.resource_key(), Some(&key));
        assert!(!err.is_retryable());
    }

    #[test]
    fn cancelled_is_not_retryable() {
        let err = Error::cancelled();
        assert!(!err.is_retryable());
        assert_eq!(*err.kind(), ErrorKind::Cancelled);
    }

    #[test]
    fn backpressure_is_retryable() {
        let err = Error::backpressure("pool full");
        assert!(err.is_retryable());
        assert_eq!(*err.kind(), ErrorKind::Backpressure);
    }

    #[test]
    fn backpressure_has_default_retry_after() {
        let err = Error::backpressure("pool full");
        assert_eq!(err.retry_after(), Some(Duration::from_millis(50)));
    }

    #[test]
    fn display_includes_resource_key() {
        let key = nebula_core::resource_key!("redis");
        let err = Error::transient("connection reset").with_resource_key(key);
        let msg = err.to_string();
        assert!(msg.contains("redis"), "expected 'redis' in: {msg}");
        assert!(msg.contains("connection reset"));
    }

    #[test]
    fn display_without_resource_key() {
        let err = Error::permanent("bad config");
        assert_eq!(err.to_string(), "bad config");
    }

    #[test]
    fn with_source_chains_error() {
        let inner = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        let err = Error::transient("connection failed").with_source(inner);
        let source = std::error::Error::source(&err);
        assert!(source.is_some());
    }

    #[test]
    fn default_scope_is_resource() {
        assert_eq!(ErrorScope::default(), ErrorScope::Resource);
    }
}
