//! Accessor trait definitions for capability injection.
//!
//! Trait definitions only -- implementations live in domain crates.

use std::{future::Future, pin::Pin, time::Instant};

use chrono::{DateTime, Utc};

/// Type alias for dyn-safe async return.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Dyn-safe resource accessor. Impl in nebula-engine.
pub trait ResourceAccessor: Send + Sync {
    /// Check if a resource is available.
    fn has(&self, key: &crate::ResourceKey) -> bool;
    /// Acquire a resource by key.
    fn acquire_any(
        &self,
        key: &crate::ResourceKey,
    ) -> BoxFuture<'_, Result<Box<dyn std::any::Any + Send + Sync>, crate::CoreError>>;
    /// Try to acquire a resource by key, returning `None` if not found.
    fn try_acquire_any(
        &self,
        key: &crate::ResourceKey,
    ) -> BoxFuture<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, crate::CoreError>>;
    /// The managed row facade of the resource `key`, type-erased.
    ///
    /// Unlike [`acquire_any`](Self::acquire_any) this checks nothing out:
    /// an implementation that serves managed rows returns the resource
    /// crate's `ManagedRow<R>` boxed, for the caller to downcast, and each
    /// unit submitted on it checks out an instance per attempt. The facade
    /// is bound to the calling context: its units are cancelled with it
    /// until their first grant and bounded by its deadline.
    ///
    /// The default serves none: it refuses with a non-retryable
    /// [`CoreError::ResourceUnavailable`](crate::CoreError::ResourceUnavailable).
    ///
    /// # Errors
    ///
    /// Whatever the implementation's lookup refuses; the default always
    /// refuses.
    fn managed_row_any(
        &self,
        key: &crate::ResourceKey,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, crate::CoreError> {
        Err(crate::CoreError::resource_unavailable(
            key.to_string(),
            "this resource accessor serves no managed rows",
            false,
            None,
        ))
    }

    /// Tries to resolve the managed row facade of resource `key`.
    ///
    /// `Ok(None)` means that no matching row exists. Lifecycle and other
    /// accessor failures remain errors so optional action slots cannot hide a
    /// temporarily unavailable or incorrectly typed row. The default serves
    /// no managed rows.
    ///
    /// # Errors
    ///
    /// Whatever the implementation's lookup refuses; the default never
    /// errors.
    fn try_managed_row_any(
        &self,
        _key: &crate::ResourceKey,
    ) -> Result<Option<Box<dyn std::any::Any + Send + Sync>>, crate::CoreError> {
        Ok(None)
    }
}

/// Dyn-safe credential accessor. Impl in nebula-engine.
pub trait CredentialAccessor: Send + Sync {
    /// Check if a credential is available.
    fn has(&self, key: &crate::CredentialKey) -> bool;
    /// Resolve a credential by key.
    fn resolve_any(
        &self,
        key: &crate::CredentialKey,
    ) -> BoxFuture<'_, Result<Box<dyn std::any::Any + Send + Sync>, crate::CoreError>>;
    /// Try to resolve a credential by key, returning `None` if not found.
    fn try_resolve_any(
        &self,
        key: &crate::CredentialKey,
    ) -> BoxFuture<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, crate::CoreError>>;
}

/// Structured logger interface.
pub trait Logger: Send + Sync {
    /// Log a message at the given level.
    fn log(&self, level: LogLevel, message: &str);
    /// Log a message with structured fields.
    fn log_with_fields(&self, level: LogLevel, message: &str, fields: &[(&str, &str)]);
}

/// Log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Trace level.
    Trace,
    /// Debug level.
    Debug,
    /// Info level.
    Info,
    /// Warn level.
    Warn,
    /// Error level.
    Error,
}

/// Metrics emission interface.
pub trait MetricsEmitter: Send + Sync {
    /// Increment a counter.
    fn counter(&self, name: &str, value: u64, labels: &[(&str, &str)]);
    /// Set a gauge value.
    fn gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]);
    /// Record a histogram value.
    fn histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}

/// Event bus emitter.
pub trait EventEmitter: Send + Sync {
    /// Emit an event to a topic.
    fn emit(&self, topic: &str, payload: serde_json::Value);
}

/// Clock abstraction for deterministic testing.
pub trait Clock: Send + Sync {
    /// Current wall-clock time.
    fn now(&self) -> DateTime<Utc>;
    /// Monotonic instant.
    fn monotonic(&self) -> Instant;
}

/// Real-time clock implementation.
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
    fn monotonic(&self) -> Instant {
        Instant::now()
    }
}

/// Single-flight refresh coordination (spec 22).
pub trait RefreshCoordinator: Send + Sync {
    /// Acquire a refresh lock for the given credential.
    ///
    /// Takes a typed [`crate::id::CredentialId`] rather than a raw `&str` so the
    /// compiler enforces that callers pass an actual credential identifier — not an
    /// arbitrary string — at every call site.
    fn acquire_refresh(
        &self,
        credential_id: &crate::id::CredentialId,
    ) -> BoxFuture<'_, Result<RefreshToken, crate::CoreError>>;
    /// Release a refresh lock.
    fn release_refresh(&self, token: RefreshToken) -> BoxFuture<'_, Result<(), crate::CoreError>>;
}

/// Token returned by [`RefreshCoordinator`].
///
/// The token is an opaque handle minted exclusively by a [`RefreshCoordinator`]
/// implementation.  Callers must pass it back to [`RefreshCoordinator::release_refresh`]
/// to release the lock.  TTL / timeout semantics are delegated to the concrete
/// implementation.
#[derive(Debug)]
pub struct RefreshToken(u64);

impl RefreshToken {
    /// Mint a new token from a coordinator-assigned value.
    #[must_use = "dropping a RefreshToken without calling release_refresh leaks the refresh lock"]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the raw coordinator-assigned value.
    #[must_use]
    pub const fn get(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{BoxFuture, RefreshToken, ResourceAccessor};
    use crate::{CoreError, ResourceKey};

    /// An accessor that implements only the required methods.
    struct LeasesOnly;

    impl ResourceAccessor for LeasesOnly {
        fn has(&self, _key: &ResourceKey) -> bool {
            true
        }

        fn acquire_any(
            &self,
            key: &ResourceKey,
        ) -> BoxFuture<'_, Result<Box<dyn std::any::Any + Send + Sync>, CoreError>> {
            let key = key.to_string();
            Box::pin(async move { Err(CoreError::resource_unavailable(key, "unused", true, None)) })
        }

        fn try_acquire_any(
            &self,
            _key: &ResourceKey,
        ) -> BoxFuture<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, CoreError>>
        {
            Box::pin(async { Ok(None) })
        }
    }

    #[test]
    fn the_default_accessor_serves_no_managed_rows() {
        use nebula_error::Classify;

        let key = ResourceKey::new("postgres").expect("valid key");
        let error = LeasesOnly
            .managed_row_any(&key)
            .expect_err("no managed rows by default");
        assert!(!error.is_retryable(), "a missing capability never heals");
        assert_eq!(error.category(), nebula_error::ErrorCategory::Unavailable);
        assert!(matches!(
            &error,
            CoreError::ResourceUnavailable { key, detail, .. }
                if key == "postgres" && detail.contains("serves no managed rows")
        ));
        assert!(
            LeasesOnly
                .try_managed_row_any(&key)
                .expect("the default try lookup is infallible")
                .is_none()
        );
    }

    #[test]
    fn refresh_token_round_trips() {
        assert_eq!(RefreshToken::new(42).get(), 42);
        assert_eq!(RefreshToken::new(99).get(), 99);
        assert_ne!(RefreshToken::new(1).get(), RefreshToken::new(2).get());
    }
}
