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

#[cfg(test)]
mod cancellation_wakeup_tests {
    //! Regression for #632: the cancellation wait is stored, not rebuilt on
    //! every poll.

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
