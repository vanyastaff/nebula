//! Unified error types for the resource subsystem.
//!
//! Every resource error carries an [`ErrorKind`] (what happened) and an
//! [`ErrorScope`] (currently resource-wide only — see the [`ErrorScope`]
//! type docs for why it is single-variant). The framework uses
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
    /// Resource tainted by a credential revoke — new acquires are
    /// rejected until the credential is re-registered.
    ///
    /// Non-terminal: the taint is lifted when the resource is
    /// re-registered with a fresh credential, so this classifies as a
    /// transient/unavailable condition (retry after a short backoff),
    /// **not** a cancellation.
    Revoked,
    /// More than one resolved-credential registration exists for the
    /// requested `(key, scope)` and the caller supplied no slot identity
    /// to disambiguate — a fail-closed deny.
    ///
    /// This is a caller/wiring fault, not an internal invariant breach:
    /// either register the resource single-tenant per `(key, scope)`, or
    /// acquire through a slot-identity-pinned path. It is a permanent
    /// caller error (never auto-retried — the caller must change how it
    /// resolves the resource), classified as a client conflict, **not** a
    /// server (5xx) failure.
    Ambiguous,
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
    /// `Transient`, `Exhausted`, `Backpressure`, and `Revoked` are
    /// retryable because they represent transient conditions that resolve
    /// with time or backoff (`Revoked` clears once the credential is
    /// re-registered).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Transient
                | ErrorKind::Exhausted { .. }
                | ErrorKind::Backpressure
                | ErrorKind::Revoked
        )
    }

    /// Returns the retry-after hint, if available.
    ///
    /// - `Exhausted` errors carry an explicit `retry_after` from the upstream.
    /// - `Backpressure` errors return a default 50ms hint (pool slots free up quickly).
    /// - `Revoked` errors return a 100ms hint (re-registration is operator-paced;
    ///   a short floor avoids a hot retry loop without stalling recovery).
    pub fn retry_after(&self) -> Option<Duration> {
        match &self.kind {
            ErrorKind::Exhausted { retry_after } => *retry_after,
            ErrorKind::Backpressure => Some(Duration::from_millis(50)),
            ErrorKind::Revoked => Some(Duration::from_millis(100)),
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

    /// Creates a revoked error (resource tainted by a credential revoke).
    ///
    /// Non-terminal — retryable with a short backoff (see
    /// [`is_retryable`](Self::is_retryable) / [`retry_after`](Self::retry_after)).
    pub fn revoked(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Revoked, message)
    }

    /// Creates an ambiguous-resolution error (more than one resolved-
    /// credential registration matched `(key, scope)` and no slot identity
    /// was supplied to disambiguate).
    ///
    /// Permanent caller error — never retried (the caller must supply a
    /// resolved slot identity or register single-tenant); classified as a
    /// client conflict, not a server failure.
    pub fn ambiguous(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Ambiguous, message)
    }

    /// Creates a backpressure error.
    pub fn backpressure(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Backpressure, message)
    }

    /// An unknown credential slot rotated.
    ///
    /// Per slot model invariant: the engine emits a slot-name to
    /// `Resource::on_credential_refresh`; the implementer must handle
    /// every declared slot. If rotation arrives for a slot that was
    /// not declared via `#[credential]`, the engine surfaces this
    /// error so the operator can correlate it with a misconfigured
    /// dependency.
    pub fn unknown_credential_slot(key: ResourceKey, slot_name: &str) -> Self {
        Self::permanent(format!(
            "{key}: rotation for unknown credential slot `{slot_name}` — \
             slot was not declared via `#[credential(key = ...)]` on the resource struct"
        ))
        .with_resource_key(key)
    }

    /// Maps this error into the accessor [`nebula_core::CoreError`] surface.
    #[must_use]
    pub fn to_core_error(&self) -> nebula_core::CoreError {
        let key_label = self
            .resource_key()
            .map(|k| k.as_str().to_owned())
            .unwrap_or_else(|| "resource".to_owned());
        let detail = self.to_string();
        match self.kind() {
            ErrorKind::NotFound => nebula_core::CoreError::CredentialNotFound { key: detail },
            ErrorKind::Ambiguous => nebula_core::CoreError::scope_violation(key_label, detail),
            ErrorKind::Cancelled => {
                nebula_core::CoreError::resource_unavailable(key_label, detail, false, None)
            },
            ErrorKind::Permanent => {
                nebula_core::CoreError::resource_unavailable(key_label, detail, false, None)
            },
            ErrorKind::Transient
            | ErrorKind::Exhausted { .. }
            | ErrorKind::Backpressure
            | ErrorKind::Revoked => nebula_core::CoreError::resource_unavailable(
                key_label,
                detail,
                true,
                self.retry_after(),
            ),
        }
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
            // Non-terminal: the taint clears on credential re-registration.
            // `Unavailable` is the retryable family the shared classifier
            // uses for "temporarily down, try again" (see
            // `ErrorCategory::is_default_retryable`).
            ErrorKind::Revoked => nebula_error::ErrorCategory::Unavailable,
            // Caller/wiring fault, not an internal breach: the caller asked
            // for a `(key, scope)` that resolves to more than one tenant's
            // registration without pinning a slot identity. `Conflict` is
            // the client-error, non-retryable family (it is NOT a server
            // 5xx — see `ErrorCategory::is_client_error` /
            // `is_server_error`), so the deny surfaces as a caller conflict
            // rather than `Internal`.
            ErrorKind::Ambiguous => nebula_error::ErrorCategory::Conflict,
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
            ErrorKind::Revoked => "RESOURCE:REVOKED",
            ErrorKind::Ambiguous => "RESOURCE:AMBIGUOUS",
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
    fn to_core_error_maps_retryable_transient() {
        use nebula_error::Classify as _;
        let key = nebula_core::resource_key!("postgres");
        let err = Error::transient("upstream timeout").with_resource_key(key);
        let core = err.to_core_error();
        assert!(matches!(
            core,
            nebula_core::CoreError::ResourceUnavailable {
                retryable: true,
                ..
            }
        ));
        assert!(core.is_retryable());
    }

    #[test]
    fn to_core_error_maps_not_found_to_credential_not_found() {
        let key = nebula_core::resource_key!("postgres");
        let err = Error::not_found(&key);
        assert!(matches!(
            err.to_core_error(),
            nebula_core::CoreError::CredentialNotFound { .. }
        ));
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
    fn revoked_is_retryable_and_not_cancelled() {
        use nebula_error::{Classify, ErrorCategory};

        let err = Error::revoked("tainted by revoke");
        // A tainted resource is acquirable again once the credential is
        // re-registered — semantically transient, NOT terminal.
        assert!(err.is_retryable());
        assert_eq!(*err.kind(), ErrorKind::Revoked);
        assert_ne!(
            Classify::category(&err),
            ErrorCategory::Cancelled,
            "Revoked must not classify as Cancelled (it is non-terminal)"
        );
        assert_eq!(Classify::category(&err), ErrorCategory::Unavailable);
        assert_eq!(Classify::code(&err).as_str(), "RESOURCE:REVOKED");
        assert_eq!(
            err.retry_after(),
            Some(Duration::from_millis(100)),
            "Revoked carries a short retry hint"
        );
        let hint = Classify::retry_hint(&err).expect("Revoked has a retry hint");
        assert_eq!(hint.after, Some(Duration::from_millis(100)));
    }

    #[test]
    fn ambiguous_is_caller_conflict_not_internal() {
        use nebula_error::{Classify, ErrorCategory};

        let err = Error::ambiguous("two resolved-credential rows; supply slot identity");
        // An ambiguous resolution is a caller/wiring fault (the caller did
        // not pin a slot identity), NOT an internal invariant breach.
        assert_eq!(*err.kind(), ErrorKind::Ambiguous);
        assert_eq!(
            Classify::category(&err),
            ErrorCategory::Conflict,
            "Ambiguous must classify as a client conflict"
        );
        assert_ne!(
            Classify::category(&err),
            ErrorCategory::Internal,
            "Ambiguous must not surface as a 5xx server error"
        );
        assert!(
            ErrorCategory::Conflict.is_client_error(),
            "Conflict must be a client error"
        );
        assert!(
            !ErrorCategory::Conflict.is_server_error(),
            "Conflict must not be a server error"
        );
        // Permanent caller error — the caller must change how it resolves
        // the resource; never auto-retried.
        assert!(
            !err.is_retryable(),
            "Ambiguous is a permanent caller error, not retryable"
        );
        assert!(
            !Classify::category(&err).is_default_retryable(),
            "Conflict must not be default-retryable"
        );
        assert_eq!(Classify::code(&err).as_str(), "RESOURCE:AMBIGUOUS");
        assert_eq!(
            err.retry_after(),
            None,
            "Ambiguous carries no retry hint (permanent caller error)"
        );
        assert!(
            Classify::retry_hint(&err).is_none(),
            "Ambiguous exposes no retry hint"
        );
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
