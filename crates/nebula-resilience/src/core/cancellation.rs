//! Cancellation support for resilience patterns
//!
//! This module provides structured cancellation handling that integrates
//! with tokio's cancellation tokens and follows the project's guidelines
//! for graceful shutdown and operation cancellation.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_util::sync::CancellationToken;

use crate::core::error::ResilienceError;
use crate::core::result::ResilienceResult;

/// Cancellation-aware operation wrapper
///
/// Provides structured cancellation support for resilience operations
/// following the project's requirements for handling shutdown signals.
#[derive(Debug, Clone)]
pub struct CancellationContext {
    /// Primary cancellation token
    token: CancellationToken,
    /// Optional reason for cancellation
    reason: Option<String>,
}

impl CancellationContext {
    /// Create a new cancellation context
    #[must_use]
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            reason: None,
        }
    }

    /// Create a cancellation context with a reason
    pub fn with_reason(reason: impl Into<String>) -> Self {
        Self {
            token: CancellationToken::new(),
            reason: Some(reason.into()),
        }
    }

    /// Create a child context that will be cancelled when parent is cancelled
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            token: self.token.child_token(),
            reason: self.reason.clone(),
        }
    }

    /// Cancel this context
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Check if cancellation has been requested
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Get the cancellation token
    #[must_use]
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Get the cancellation reason if available
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Execute an operation with cancellation support
    ///
    /// This follows the pattern from .cursorrules:
    /// ```text
    /// tokio::select! {
    ///     result = operation() => result,
    ///     _ = shutdown.cancelled() => Err(Cancelled)
    /// }
    /// ```
    #[tracing::instrument(skip(self, operation), fields(
        cancellation_reason = self.reason.as_deref().unwrap_or("none")
    ))]
    pub async fn execute<F, Fut, T>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        tokio::select! {
            result = operation() => {
                tracing::debug!("Operation completed before cancellation");
                result
            }
            () = self.token.cancelled() => {
                tracing::info!("Operation cancelled");
                Err(ResilienceError::Cancelled {
                    reason: self.reason.clone(),
                })
            }
        }
    }

    /// Execute with timeout and cancellation
    #[tracing::instrument(skip(self, operation), fields(
        timeout_ms = timeout.as_millis(),
        cancellation_reason = self.reason.as_deref().unwrap_or("none")
    ))]
    pub async fn execute_with_timeout<F, Fut, T>(
        &self,
        operation: F,
        timeout: std::time::Duration,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        tokio::select! {
            result = tokio::time::timeout(timeout, operation()) => {
                if let Ok(op_result) = result {
                    tracing::debug!("Operation completed within timeout");
                    op_result
                } else {
                    tracing::warn!(?timeout, "Operation timed out");
                    Err(ResilienceError::Timeout {
                        duration: timeout,
                        context: Some("Operation exceeded timeout".to_string()),
                    })
                }
            }
            () = self.token.cancelled() => {
                tracing::info!("Operation cancelled before timeout");
                Err(ResilienceError::Cancelled {
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

/// Future wrapper that can be cancelled
pub struct CancellableFuture<F> {
    future: Pin<Box<F>>,
    cancellation: CancellationToken,
}

impl<F> CancellableFuture<F>
where
    F: Future,
{
    /// Create a new cancellable future
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
    type Output = Result<F::Output, ResilienceError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check for cancellation first
        if self.cancellation.is_cancelled() {
            return Poll::Ready(Err(ResilienceError::Cancelled {
                reason: Some("Future was cancelled".to_string()),
            }));
        }

        // Poll the underlying future
        match self.future.as_mut().poll(cx) {
            Poll::Ready(output) => Poll::Ready(Ok(output)),
            Poll::Pending => {
                // Register for cancellation notifications
                let cancellation_future = self.cancellation.cancelled();
                tokio::pin!(cancellation_future);

                // Check if cancellation is ready
                if cancellation_future.as_mut().poll(cx).is_ready() {
                    Poll::Ready(Err(ResilienceError::Cancelled {
                        reason: Some("Future was cancelled while pending".to_string()),
                    }))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// Extension trait for adding cancellation support to futures
pub trait CancellationExt<T>: Future<Output = T> + Sized {
    /// Add cancellation support to this future
    fn with_cancellation(self, token: CancellationToken) -> CancellableFuture<Self> {
        CancellableFuture::new(self, token)
    }
}

impl<F, T> CancellationExt<T> for F where F: Future<Output = T> {}

/// Global shutdown coordinator for the application
#[derive(Debug, Clone)]
pub struct ShutdownCoordinator {
    /// Master cancellation token
    master_token: CancellationToken,
    /// Graceful shutdown timeout
    graceful_timeout: std::time::Duration,
}

impl ShutdownCoordinator {
    /// Create a new shutdown coordinator
    #[must_use]
    pub fn new(graceful_timeout: std::time::Duration) -> Self {
        Self {
            master_token: CancellationToken::new(),
            graceful_timeout,
        }
    }

    /// Create a cancellation context for operations
    #[must_use]
    pub fn create_context(&self, reason: Option<String>) -> CancellationContext {
        CancellationContext {
            token: self.master_token.child_token(),
            reason,
        }
    }

    /// Initiate graceful shutdown
    #[tracing::instrument(skip(self))]
    pub async fn shutdown(&self) {
        tracing::info!("Initiating graceful shutdown");
        self.master_token.cancel();

        // Wait for graceful timeout
        tokio::time::sleep(self.graceful_timeout).await;
        tracing::warn!("Graceful shutdown timeout exceeded");
    }

    /// Check if shutdown has been initiated
    #[must_use]
    pub fn is_shutdown_requested(&self) -> bool {
        self.master_token.is_cancelled()
    }

    /// Get the master cancellation token
    #[must_use]
    pub fn token(&self) -> &CancellationToken {
        &self.master_token
    }
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new(std::time::Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_cancellation_context() {
        let ctx = CancellationContext::new();

        // Test normal operation
        let result = ctx
            .execute(|| async { Ok::<i32, ResilienceError>(42) })
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_cancellation_during_operation() {
        let ctx = CancellationContext::new();
        let ctx_clone = ctx.clone();

        // Cancel after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ctx_clone.cancel();
        });

        // Long-running operation
        let result = ctx
            .execute(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<i32, ResilienceError>(42)
            })
            .await;

        assert!(result.is_err());
        if let Err(ResilienceError::Cancelled { .. }) = result {
            // Expected
        } else {
            panic!("Expected cancellation error");
        }
    }

    #[tokio::test]
    async fn test_timeout_with_cancellation() {
        let ctx = CancellationContext::with_reason("test");

        // Operation that times out
        let result = ctx
            .execute_with_timeout(
                || async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<i32, ResilienceError>(42)
                },
                Duration::from_millis(10),
            )
            .await;

        assert!(result.is_err());
        if let Err(ResilienceError::Timeout { .. }) = result {
            // Expected timeout
        } else {
            panic!("Expected timeout error, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_shutdown_coordinator() {
        let coordinator = ShutdownCoordinator::new(Duration::from_millis(100));
        let ctx = coordinator.create_context(Some("test operation".to_string()));

        assert!(!coordinator.is_shutdown_requested());

        // Start shutdown in background
        let coordinator_clone = coordinator.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            coordinator_clone.shutdown().await;
        });

        // Operation should be cancelled
        let result = ctx
            .execute(|| async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<i32, ResilienceError>(42)
            })
            .await;

        assert!(result.is_err());
        assert!(coordinator.is_shutdown_requested());
    }

    #[tokio::test]
    async fn test_child_context_cancellation() {
        let parent = CancellationContext::new();
        let child = parent.child();

        // Start a task that will be cancelled
        let child_clone = child.clone();
        let task = tokio::spawn(async move {
            child_clone
                .execute(|| async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<i32, ResilienceError>(42)
                })
                .await
        });

        // Give a small delay then cancel parent
        tokio::time::sleep(Duration::from_millis(10)).await;
        parent.cancel();

        // The task should be cancelled
        let result = task.await.unwrap();
        assert!(result.is_err());
        if let Err(ResilienceError::Cancelled { .. }) = result {
            // Expected
        } else {
            panic!("Expected cancellation error, got: {:?}", result);
        }
    }
}
