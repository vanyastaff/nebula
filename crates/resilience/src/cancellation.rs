//! Cancellation support for resilience patterns.
//!
//! Provides structured cancellation handling that integrates
//! with tokio's cancellation tokens for graceful shutdown and operation cancellation.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_util::sync::CancellationToken;

use crate::CallError;

/// Cancellation-aware operation wrapper.
///
/// Provides structured cancellation support for resilience operations.
#[derive(Debug, Clone)]
pub struct CancellationContext {
    /// Primary cancellation token
    token: CancellationToken,
    /// Optional reason for cancellation
    reason: Option<String>,
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
    pub fn with_reason(reason: impl Into<String>) -> Self {
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

    /// Execute an operation with cancellation support.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if the cancellation token fires
    /// before the operation completes. Propagates any `CallError` returned by `operation`.
    #[tracing::instrument(skip(self, operation), fields(
        cancellation_reason = self.reason.as_deref().unwrap_or("none")
    ))]
    pub async fn execute<F, Fut, T, E>(&self, operation: F) -> Result<T, CallError<E>>
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

    /// Execute with timeout and cancellation.
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
    pub async fn execute_with_timeout<F, Fut, T, E>(
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

/// Future wrapper that can be cancelled.
///
/// Polls both the inner future and the cancellation token concurrently.
/// If cancellation fires before the inner future completes, returns
/// `Err(CallError::Cancelled)`.
///
/// The cancellation future is created once at construction and reused across
/// polls — no per-poll allocation.
pub struct CancellableFuture<F> {
    future: Pin<Box<F>>,
    /// We use `tokio::select!` internally via a helper that owns the token.
    cancellation: CancellationToken,
}

impl<F> CancellableFuture<F>
where
    F: Future,
{
    /// Create a new cancellable future.
    pub fn new(future: F, cancellation: CancellationToken) -> Self {
        Self {
            future: Box::pin(future),
            cancellation,
        }
    }
}

impl<F> Future for CancellableFuture<F>
where
    F: Future,
{
    type Output = Result<F::Output, CallError<()>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Fast path: check if already cancelled (no allocation)
        if self.cancellation.is_cancelled() {
            return Poll::Ready(Err(CallError::Cancelled {
                reason: Some("Future was cancelled".to_string()),
            }));
        }

        // Poll the underlying future
        match self.future.as_mut().poll(cx) {
            Poll::Ready(output) => Poll::Ready(Ok(output)),
            Poll::Pending => {
                // `is_cancelled()` + waker registration: CancellationToken
                // internally registers the waker when polled via `cancelled()`.
                // We create a short-lived future just to register the waker.
                // This is cheap — `cancelled()` is a thin wrapper that checks
                // an atomic and registers the waker, no heap allocation.
                let waker_future = self.cancellation.cancelled();
                tokio::pin!(waker_future);
                if waker_future.as_mut().poll(cx).is_ready() {
                    Poll::Ready(Err(CallError::Cancelled {
                        reason: Some("Future was cancelled while pending".to_string()),
                    }))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// Extension trait for adding cancellation support to futures.
pub trait CancellationExt<T>: Future<Output = T> + Sized {
    /// Add cancellation support to this future.
    fn with_cancellation(self, token: CancellationToken) -> CancellableFuture<Self> {
        CancellableFuture::new(self, token)
    }
}

impl<F, T> CancellationExt<T> for F where F: Future<Output = T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_cancellation_context() {
        let ctx = CancellationContext::new();

        let result = ctx
            .execute(|| async { Ok::<i32, CallError<&str>>(42) })
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_cancellation_during_operation() {
        let ctx = CancellationContext::new();
        let ctx_clone = ctx.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ctx_clone.cancel();
        });

        let result: Result<i32, CallError<&str>> = ctx
            .execute(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(42)
            })
            .await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CallError::Cancelled { .. })));
    }

    #[tokio::test]
    async fn test_timeout_with_cancellation() {
        let ctx = CancellationContext::with_reason("test");

        let result: Result<i32, CallError<&str>> = ctx
            .execute_with_timeout(
                || async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok(42)
                },
                Duration::from_millis(10),
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result, Err(CallError::Timeout(_))));
    }

    #[tokio::test]
    async fn test_child_context_cancellation() {
        let parent = CancellationContext::new();
        let child = parent.child();

        let child_clone = child.clone();
        let task = tokio::spawn(async move {
            child_clone
                .execute(|| async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<i32, CallError<&str>>(42)
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        parent.cancel();

        let result = task.await.unwrap();
        assert!(result.is_err());
        assert!(matches!(result, Err(CallError::Cancelled { .. })));
    }
}
