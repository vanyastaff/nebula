//! Cancellation support for resilience patterns.
//!
//! Provides structured cancellation handling that integrates
//! with tokio's cancellation tokens for graceful shutdown and operation cancellation.

use std::{
    borrow::Cow,
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

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

/// Future wrapper that can be cancelled.
///
/// Polls both the inner future and the cancellation token concurrently.
/// If cancellation fires before the inner future completes, returns
/// `Err(CallError::Cancelled)`.
///
/// The cancellation future is created once at construction and reused across
/// polls — no per-poll allocation.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{CallError, CancellationExt};
/// use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let token = CancellationToken::new();
/// let work = async { 42_u32 };
///
/// let cancellable = work.with_cancellation(token.clone());
/// token.cancel();
///
/// let result: Result<u32, CallError<()>> = cancellable.await;
/// assert!(matches!(result, Err(CallError::Cancelled { .. })));
/// # Ok(())
/// # }
/// ```
pub struct CancellableFuture<F> {
    future: Pin<Box<F>>,
    /// The cancellation wait, created **once** and polled across every poll of
    /// this future.
    ///
    /// `CancellationToken::cancelled()` borrows the token, so storing that
    /// future beside the token it borrows would be self-referential — which is
    /// why the previous implementation rebuilt it on every poll instead. Each
    /// rebuild re-registers a waker with the token, so a frequently-yielding
    /// inner future paid that cost once per yield. `cancelled_owned()` holds
    /// its own clone of the token, so the wait can simply be a field.
    cancellation: Pin<Box<WaitForCancellationFutureOwned>>,
    /// Retained for the fast path and for `is_cancelled` checks; the wait above
    /// owns its own clone.
    token: CancellationToken,
}

impl<F> fmt::Debug for CancellableFuture<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellableFuture").finish_non_exhaustive()
    }
}

impl<F> CancellableFuture<F>
where
    F: Future,
{
    /// Create a new cancellable future.
    #[must_use]
    pub fn new(future: F, cancellation: CancellationToken) -> Self {
        Self {
            future: Box::pin(future),
            cancellation: Box::pin(cancellation.clone().cancelled_owned()),
            token: cancellation,
        }
    }
}

impl<F> Future for CancellableFuture<F>
where
    F: Future,
{
    type Output = Result<F::Output, CallError<()>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Fast path: already cancelled before the inner future ever ran.
        if self.token.is_cancelled() {
            return Poll::Ready(Err(CallError::cancelled()));
        }

        // Poll the underlying future first: work that is already complete is
        // reported as complete, even if cancellation arrives in the same wake.
        match self.future.as_mut().poll(cx) {
            Poll::Ready(output) => Poll::Ready(Ok(output)),
            Poll::Pending => {
                // One stored wait, polled across every poll of this future, so
                // the waker is registered once rather than once per yield.
                if self.cancellation.as_mut().poll(cx).is_ready() {
                    Poll::Ready(Err(CallError::cancelled_with(
                        "Future was cancelled while pending",
                    )))
                } else {
                    Poll::Pending
                }
            },
        }
    }
}

/// Extension trait for adding cancellation support to futures.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{CallError, CancellationExt};
/// use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let token = CancellationToken::new();
/// let value: Result<u32, CallError<()>> = async { 42 }.with_cancellation(token).await;
/// assert_eq!(value.unwrap(), 42);
/// # Ok(())
/// # }
/// ```
pub trait CancellationExt<T>: Future<Output = T> + Sized {
    /// Add cancellation support to this future.
    fn with_cancellation(self, token: CancellationToken) -> CancellableFuture<Self> {
        CancellableFuture::new(self, token)
    }
}

impl<F, T> CancellationExt<T> for F where F: Future<Output = T> {}

#[cfg(test)]
mod cancellation_wakeup_tests {
    //! Regression for #632: the cancellation wait is created once and stored,
    //! not rebuilt on every poll.

    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::{CallError, CancellationExt};

    /// A future that yields many times must still be woken by cancellation.
    ///
    /// Storing the wait instead of rebuilding it changes *when* the waker is
    /// registered, so this pins the property that matters: after any number of
    /// yields, cancelling still completes the future promptly.
    #[tokio::test]
    async fn cancellation_wakes_a_frequently_yielding_future() {
        let token = CancellationToken::new();
        let canceller = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            canceller.cancel();
        });

        let yielding = async {
            loop {
                tokio::task::yield_now().await;
            }
        };

        let outcome: Result<(), CallError<()>> =
            tokio::time::timeout(Duration::from_secs(5), yielding.with_cancellation(token))
                .await
                .expect("cancellation must wake the future long before this bound");

        assert!(
            matches!(outcome, Err(CallError::Cancelled { .. })),
            "a cancelled yielding future must report Cancelled, got {outcome:?}"
        );
    }

    /// Completion still wins when the inner future is already done.
    #[tokio::test]
    async fn a_ready_future_completes_even_after_many_yields() {
        let token = CancellationToken::new();
        let work = async {
            for _ in 0..10_000 {
                tokio::task::yield_now().await;
            }
            7_u32
        };

        let outcome: Result<u32, CallError<()>> = work.with_cancellation(token).await;
        assert_eq!(
            outcome.expect("an uncancelled future must complete"),
            7,
            "repeated yields must not change the result"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_cancellation_context() {
        let ctx = CancellationContext::new();

        let result = ctx.call(|| async { Ok::<i32, CallError<&str>>(42) }).await;
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
            .call(|| async {
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
            .call_with_timeout(
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
                .call(|| async {
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
