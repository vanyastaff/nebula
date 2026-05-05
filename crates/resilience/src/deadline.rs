//! Shared deadline helper for policies that need a total time budget.

use std::{
    future::Future,
    time::{Duration, Instant},
};

use crate::CallError;

/// A monotonic deadline represented as a start instant plus total budget.
///
/// This is intentionally small and copyable. It does not replace the injectable
/// [`Clock`](crate::clock::Clock) used by state machines, but it gives async policies
/// one shared way to enforce "remaining budget" for attempts and sleeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    start: Instant,
    budget: Duration,
}

impl Deadline {
    /// Create a deadline starting at `Instant::now()`.
    #[must_use]
    pub fn after(budget: Duration) -> Self {
        Self::from_start(Instant::now(), budget)
    }

    /// Create a deadline from an explicit start instant.
    #[must_use]
    pub const fn from_start(start: Instant, budget: Duration) -> Self {
        Self { start, budget }
    }

    /// Total configured budget.
    #[must_use]
    pub const fn budget(self) -> Duration {
        self.budget
    }

    /// Elapsed time since the deadline start.
    #[must_use]
    pub fn elapsed(self) -> Duration {
        self.start.elapsed()
    }

    /// Remaining time, if any.
    #[must_use]
    pub fn remaining(self) -> Option<Duration> {
        self.budget.checked_sub(self.elapsed())
    }

    /// Remaining time or `CallError::Timeout` if the deadline has expired.
    ///
    /// # Errors
    ///
    /// Returns `CallError::Timeout` when no positive budget remains.
    pub fn remaining_or_timeout<E>(self) -> Result<Duration, CallError<E>> {
        self.remaining()
            .filter(|remaining| !remaining.is_zero())
            .ok_or(CallError::Timeout(self.budget))
    }

    /// Run a future within the remaining deadline.
    ///
    /// # Errors
    ///
    /// Returns `CallError::Timeout` if the deadline is already expired or the
    /// future does not complete within the remaining budget.
    pub async fn timeout<T, E, Fut>(self, future: Fut) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = T> + Send,
    {
        let remaining = self.remaining_or_timeout()?;
        tokio::time::timeout(remaining, future)
            .await
            .map_err(|_| CallError::Timeout(self.budget))
    }

    /// Sleep for `delay`, failing if that would exceed the remaining deadline.
    ///
    /// # Errors
    ///
    /// Returns `CallError::Timeout` if the deadline is already expired or `delay`
    /// is longer than the remaining budget.
    pub async fn sleep<E>(self, delay: Duration) -> Result<(), CallError<E>> {
        if delay.is_zero() {
            return Ok(());
        }

        let remaining = self.remaining_or_timeout()?;
        if delay > remaining {
            return Err(CallError::Timeout(self.budget));
        }

        self.timeout(tokio::time::sleep(delay)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sleep_rejects_delay_past_deadline() {
        let deadline = Deadline::after(Duration::from_millis(1));
        let err = deadline
            .sleep::<()>(Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, CallError::Timeout(_)));
    }

    #[tokio::test]
    async fn timeout_bounds_hung_future() {
        let deadline = Deadline::after(Duration::from_millis(1));
        let err = deadline
            .timeout::<(), (), _>(async {
                tokio::time::sleep(Duration::from_secs(1)).await;
            })
            .await
            .unwrap_err();
        assert!(matches!(err, CallError::Timeout(_)));
    }
}
