//! Cancellation support for resilience patterns.
//!
//! Provides structured cancellation handling that integrates
//! with tokio's cancellation tokens for graceful shutdown and operation cancellation.

use std::{borrow::Cow, future::Future};

use tokio_util::sync::CancellationToken;

use crate::CallError;

/// Cancellation-aware operation wrapper.
///
/// Provides structured cancellation support for resilience operations.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{CallError, CancellationContext};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let ctx = CancellationContext::with_reason("shutdown");
/// let child = ctx.child();
///
/// // Cancelling the parent propagates to the child.
/// ctx.cancel();
/// assert!(child.is_cancelled());
///
/// let result: Result<i32, CallError<&str>> = child.call(|| async { Ok(1) }).await;
/// assert!(matches!(result, Err(CallError::Cancelled { .. })));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CancellationContext {
    /// Primary cancellation token
    token: CancellationToken,
    /// Optional reason for cancellation.
    /// `Cow` avoids cloning when creating child contexts with static reasons.
    reason: Option<Cow<'static, str>>,
}

impl CancellationContext {
    /// Create a new cancellation context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            reason: None,
        }
    }

    /// Create a cancellation context with a reason.
    pub fn with_reason(reason: impl Into<Cow<'static, str>>) -> Self {
        Self {
            token: CancellationToken::new(),
            reason: Some(reason.into()),
        }
    }

    /// Create a child context that will be cancelled when parent is cancelled.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            token: self.token.child_token(),
            reason: self.reason.clone(),
        }
    }

    /// Cancel this context.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Check if cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Get the cancellation token.
    #[must_use]
    pub const fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Get the cancellation reason if available.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub(crate) fn cancelled_error<E>(&self) -> CallError<E> {
        CallError::Cancelled {
            reason: self.reason.clone(),
        }
    }

    /// Call an operation with cancellation support.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if the cancellation token fires
    /// before the operation completes. Propagates any `CallError` returned by `operation`.
    #[tracing::instrument(skip(self, operation), fields(
        cancellation_reason = self.reason.as_deref().unwrap_or("none")
    ))]
    pub async fn call<F, Fut, T, E>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, CallError<E>>>,
    {
        tokio::select! {
            result = operation() => {
                tracing::debug!("Operation completed before cancellation");
                result
            }
            () = self.token.cancelled() => {
                tracing::info!("Operation cancelled");
                Err(CallError::Cancelled {
                    reason: self.reason.clone(),
                })
            }
        }
    }

    /// Call with timeout and cancellation.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Timeout)` if the operation exceeds `timeout`.
    /// Returns `Err(CallError::Cancelled)` if cancellation fires first.
    /// Propagates any `CallError` returned by `operation`.
    #[tracing::instrument(skip(self, operation), fields(
        timeout_ms = timeout.as_millis(),
        cancellation_reason = self.reason.as_deref().unwrap_or("none")
    ))]
    pub async fn call_with_timeout<F, Fut, T, E>(
        &self,
        operation: F,
        timeout: std::time::Duration,
    ) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, CallError<E>>>,
    {
        tokio::select! {
            result = tokio::time::timeout(timeout, operation()) => {
                result.map_or_else(
                    |_| {
                        tracing::warn!(?timeout, "Operation timed out");
                        Err(CallError::Timeout(timeout))
                    },
                    |op_result| {
                        tracing::debug!("Operation completed within timeout");
                        op_result
                    },
                )
            }
            () = self.token.cancelled() => {
                tracing::info!("Operation cancelled before timeout");
                Err(CallError::Cancelled {
                    reason: self.reason.clone(),
                })
            }
        }
    }
}

impl Default for CancellationContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait for adding cancellation support to futures.
///
/// The returned future completes with `Ok` when the inner future finishes
/// first and with `Err(CallError::Cancelled)` when the token fires first. It
/// delegates to
/// [`CancellationToken::run_until_cancelled_owned`](tokio_util::sync::CancellationToken::run_until_cancelled_owned),
/// which stores the cancellation wait rather than rebuilding it on every poll.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{CallError, CancellationExt};
/// use tokio_util::sync::CancellationToken;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let token = CancellationToken::new();
/// let value: Result<u32, CallError<()>> = async { 42 }.with_cancellation(token).await;
/// assert_eq!(value.unwrap(), 42);
/// # Ok(())
/// # }
/// ```
pub trait CancellationExt<T>: Future<Output = T> + Sized + Send {
    /// Add cancellation support to this future.
    fn with_cancellation(
        self,
        token: CancellationToken,
    ) -> impl Future<Output = Result<T, CallError<()>>> + Send {
        async move {
            token
                .run_until_cancelled_owned(self)
                .await
                .ok_or_else(CallError::cancelled)
        }
    }
}

impl<F, T> CancellationExt<T> for F where F: Future<Output = T> + Send {}
