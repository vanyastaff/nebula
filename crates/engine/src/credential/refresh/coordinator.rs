//! Outer two-tier refresh coordinator.
//!
//! **Stage 2.1 stub.** Until the full L1+L2 implementation lands in
//! Stage 2.2, this module exposes a thin wrapper around the L1 coalescer
//! preserving the historical `RefreshCoordinator` public surface so that
//! the rename in 2.1 is atomic and the workspace continues to compile.
//!
//! The full coordinator (with `RefreshClaimRepo`, `RefreshCoordConfig`,
//! `refresh_coalesced`, heartbeat task, etc.) replaces this stub in 2.2.

use std::fmt;

use super::l1::{L1RefreshCoalescer, RefreshAttempt as L1Attempt, RefreshConfigError as L1Error};

/// Two-tier credential refresh coordinator (L1 in-process + L2 cross-replica).
///
/// Stage 2.1 stub: currently delegates every call to the L1 coalescer.
/// Stage 2.2 wraps it with a durable L2 claim via `RefreshClaimRepo`.
pub struct RefreshCoordinator {
    l1: L1RefreshCoalescer,
}

impl fmt::Debug for RefreshCoordinator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshCoordinator")
            .field("l1", &self.l1)
            .finish()
    }
}

/// Result of attempting to begin a refresh for a credential.
///
/// Re-exported for callers of the L1 coalescing API (Stage 2.1).
#[derive(Debug)]
pub enum RefreshAttempt {
    /// This caller won the race; perform the refresh, then call
    /// `RefreshCoordinator::complete()` to wake waiters.
    Winner,
    /// Another caller is already refreshing. Await the receiver; it
    /// resolves once the winner completes.
    Waiter(tokio::sync::oneshot::Receiver<()>),
}

impl From<L1Attempt> for RefreshAttempt {
    fn from(attempt: L1Attempt) -> Self {
        match attempt {
            L1Attempt::Winner => RefreshAttempt::Winner,
            L1Attempt::Waiter(rx) => RefreshAttempt::Waiter(rx),
        }
    }
}

/// Configuration errors for `RefreshCoordinator` constructors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshConfigError {
    /// `max_concurrent` must be at least `1`.
    #[error("RefreshCoordinator::max_concurrent must be >= 1, got 0")]
    ZeroConcurrency,
}

impl From<L1Error> for RefreshConfigError {
    fn from(err: L1Error) -> Self {
        match err {
            L1Error::ZeroConcurrency => RefreshConfigError::ZeroConcurrency,
        }
    }
}

impl RefreshCoordinator {
    /// Creates a new coordinator with default concurrency (32).
    #[must_use]
    pub fn new() -> Self {
        Self {
            l1: L1RefreshCoalescer::new(),
        }
    }

    /// Creates a new coordinator with a custom concurrency limit.
    ///
    /// # Errors
    ///
    /// Returns `RefreshConfigError::ZeroConcurrency` if `max == 0`.
    pub fn with_max_concurrent(max: usize) -> Result<Self, RefreshConfigError> {
        Ok(Self {
            l1: L1RefreshCoalescer::with_max_concurrent(max)?,
        })
    }

    /// Attempts to begin a refresh for the given credential.
    pub fn try_refresh(&self, credential_id: &str) -> RefreshAttempt {
        self.l1.try_refresh(credential_id).into()
    }

    /// Removes the in-flight entry and wakes every waiter.
    pub fn complete(&self, credential_id: &str) {
        self.l1.complete(credential_id);
    }

    /// Number of credentials currently being refreshed.
    pub fn in_flight_count(&self) -> usize {
        self.l1.in_flight_count()
    }

    /// Acquires a permit from the global concurrency limiter.
    pub async fn acquire_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.l1.acquire_permit().await
    }

    /// Available permits in the concurrency limiter.
    pub fn available_permits(&self) -> usize {
        self.l1.available_permits()
    }

    /// Records a refresh failure for circuit breaker tracking.
    pub fn record_failure(&self, credential_id: &str) {
        self.l1.record_failure(credential_id);
    }

    /// Records a successful refresh, resetting the circuit breaker.
    pub fn record_success(&self, credential_id: &str) {
        self.l1.record_success(credential_id);
    }

    /// Returns `true` if the per-credential circuit breaker is open.
    pub fn is_circuit_open(&self, credential_id: &str) -> bool {
        self.l1.is_circuit_open(credential_id)
    }
}

impl Default for RefreshCoordinator {
    fn default() -> Self {
        Self::new()
    }
}
