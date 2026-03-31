//! Refresh coordination -- prevents thundering herd on credential refresh.
//!
//! When multiple callers need a refreshed credential simultaneously, only
//! one performs the actual refresh while others wait. This avoids redundant
//! network calls and token invalidation races.
//!
//! # Design
//!
//! Per-credential refresh state is tracked in a `HashMap` behind a `Mutex`.
//! The first caller to request a refresh for a given credential ID becomes
//! the "winner" and performs the refresh. Subsequent callers receive a
//! [`Notify`] handle and wait until the winner completes.
//!
//! The winner **must** call [`RefreshCoordinator::complete()`] when done
//! (success or failure) to wake waiters and remove the in-flight entry.
//!
//! # Examples
//!
//! ```ignore
//! use nebula_credential::refresh::{RefreshCoordinator, RefreshAttempt};
//!
//! let coord = RefreshCoordinator::new();
//!
//! match coord.try_refresh("cred-1").await {
//!     RefreshAttempt::Winner(_notify) => {
//!         // Perform refresh...
//!         coord.complete("cred-1").await;
//!     }
//!     RefreshAttempt::Waiter(notify) => {
//!         notify.notified().await;
//!         // Re-read credential from store
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome};
use tokio::sync::{Mutex, Notify};

/// Coordinates credential refresh to prevent thundering herd.
///
/// Thread-safe: all operations acquire the inner `Mutex`.
///
/// Includes a per-credential circuit breaker (via `nebula_resilience::CircuitBreaker`):
/// after 5 consecutive failures the circuit opens for 5 minutes, then transitions
/// to half-open to probe recovery.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::refresh::{RefreshCoordinator, RefreshAttempt};
///
/// let coord = RefreshCoordinator::new();
///
/// // First caller wins
/// assert!(matches!(
///     coord.try_refresh("cred-1").await,
///     RefreshAttempt::Winner(_),
/// ));
///
/// // Second caller waits
/// assert!(matches!(
///     coord.try_refresh("cred-1").await,
///     RefreshAttempt::Waiter(_),
/// ));
///
/// coord.complete("cred-1").await;
/// ```
pub struct RefreshCoordinator {
    in_flight: Mutex<HashMap<String, Arc<Notify>>>,
    /// Per-credential circuit breakers via `nebula-resilience`.
    circuit_breakers: parking_lot::Mutex<HashMap<String, Arc<CircuitBreaker>>>,
}

/// Result of attempting to begin a refresh for a credential.
#[derive(Debug)]
pub enum RefreshAttempt {
    /// This caller won the race -- should perform the refresh,
    /// then call [`RefreshCoordinator::complete()`].
    ///
    /// Carries the [`Notify`] handle so the caller can wrap it in a
    /// `scopeguard` to guarantee waiters are woken on any exit path
    /// (panic, timeout, error).
    Winner(
        /// Handle to notify waiters when refresh completes.
        Arc<Notify>,
    ),
    /// Another caller is already refreshing -- wait on the [`Notify`].
    Waiter(
        /// Handle to wait on until the winner completes.
        Arc<Notify>,
    ),
}

impl RefreshCoordinator {
    /// Creates a new coordinator with no in-flight refreshes.
    pub fn new() -> Self {
        Self {
            in_flight: Mutex::new(HashMap::new()),
            circuit_breakers: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// Returns or creates a per-credential circuit breaker.
    fn get_or_create_cb(&self, credential_id: &str) -> Arc<CircuitBreaker> {
        let mut cbs = self.circuit_breakers.lock();
        cbs.entry(credential_id.to_string())
            .or_insert_with(|| {
                let config = CircuitBreakerConfig {
                    failure_threshold: 5,
                    reset_timeout: Duration::from_secs(300),
                    min_operations: 1,
                    ..Default::default()
                };
                Arc::new(CircuitBreaker::new(config).expect("valid CB config"))
            })
            .clone()
    }

    /// Attempts to begin a refresh for the given credential.
    ///
    /// Returns [`RefreshAttempt::Winner`] if this is the first caller
    /// (should perform the refresh), or [`RefreshAttempt::Waiter`] if
    /// another refresh is already in progress (should wait on the
    /// returned [`Notify`]).
    pub async fn try_refresh(&self, credential_id: &str) -> RefreshAttempt {
        let mut map = self.in_flight.lock().await;
        if let Some(notify) = map.get(credential_id) {
            RefreshAttempt::Waiter(Arc::clone(notify))
        } else {
            let notify = Arc::new(Notify::new());
            map.insert(credential_id.to_string(), Arc::clone(&notify));
            RefreshAttempt::Winner(notify)
        }
    }

    /// Called by the winner after refresh completes (success or failure).
    ///
    /// Wakes all waiters and removes the in-flight entry. **Must** be
    /// called even on error to prevent waiters from hanging indefinitely.
    pub async fn complete(&self, credential_id: &str) {
        let mut map = self.in_flight.lock().await;
        if let Some(notify) = map.remove(credential_id) {
            notify.notify_waiters();
        }
    }

    /// Returns the number of credentials currently being refreshed.
    ///
    /// Primarily useful for testing and diagnostics.
    pub async fn in_flight_count(&self) -> usize {
        self.in_flight.lock().await.len()
    }

    /// Records a refresh failure for circuit breaker tracking.
    pub async fn record_failure(&self, credential_id: &str) {
        self.get_or_create_cb(credential_id)
            .record_outcome(Outcome::Failure);
    }

    /// Records a successful refresh, resetting the circuit breaker.
    pub async fn record_success(&self, credential_id: &str) {
        // Remove the CB entirely on success — full reset.
        self.circuit_breakers.lock().remove(credential_id);
    }

    /// Returns `true` if the circuit breaker is open (too many failures).
    pub async fn is_circuit_open(&self, credential_id: &str) -> bool {
        let cbs = self.circuit_breakers.lock();
        cbs.get(credential_id)
            .is_some_and(|cb| cb.try_acquire::<()>().is_err())
    }
}

impl Default for RefreshCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_caller_wins() {
        let coord = RefreshCoordinator::new();
        let attempt = coord.try_refresh("cred-1").await;
        assert!(matches!(attempt, RefreshAttempt::Winner(_)));
        assert_eq!(coord.in_flight_count().await, 1);
    }

    #[tokio::test]
    async fn second_caller_waits() {
        let coord = RefreshCoordinator::new();
        let first = coord.try_refresh("cred-1").await;
        assert!(matches!(first, RefreshAttempt::Winner(_)));

        let second = coord.try_refresh("cred-1").await;
        assert!(matches!(second, RefreshAttempt::Waiter(_)));
    }

    #[tokio::test]
    async fn complete_removes_in_flight_entry() {
        let coord = RefreshCoordinator::new();
        let _ = coord.try_refresh("cred-1").await;
        assert_eq!(coord.in_flight_count().await, 1);

        coord.complete("cred-1").await;
        assert_eq!(coord.in_flight_count().await, 0);
    }

    #[tokio::test]
    async fn complete_wakes_waiters() {
        let coord = Arc::new(RefreshCoordinator::new());

        // Winner starts refresh
        let attempt = coord.try_refresh("cred-1").await;
        assert!(matches!(attempt, RefreshAttempt::Winner(_)));

        // Waiter gets Notify
        let waiter_attempt = coord.try_refresh("cred-1").await;
        let notify = match waiter_attempt {
            RefreshAttempt::Waiter(n) => n,
            RefreshAttempt::Winner(_) => panic!("expected Waiter"),
        };

        // Use a oneshot to signal the waiter task is ready
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

        let waiter = tokio::spawn(async move {
            let notified = notify.notified();
            // Signal that we've registered interest
            let _ = ready_tx.send(());
            notified.await;
            true
        });

        // Wait for waiter to register
        ready_rx.await.unwrap();

        // Winner completes
        coord.complete("cred-1").await;

        // Waiter should have been woken
        let result = waiter.await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn independent_credentials_do_not_interfere() {
        let coord = RefreshCoordinator::new();

        let a = coord.try_refresh("cred-a").await;
        let b = coord.try_refresh("cred-b").await;

        // Both should win -- different credentials
        assert!(matches!(a, RefreshAttempt::Winner(_)));
        assert!(matches!(b, RefreshAttempt::Winner(_)));
        assert_eq!(coord.in_flight_count().await, 2);

        coord.complete("cred-a").await;
        assert_eq!(coord.in_flight_count().await, 1);

        coord.complete("cred-b").await;
        assert_eq!(coord.in_flight_count().await, 0);
    }

    #[tokio::test]
    async fn complete_on_unknown_credential_is_noop() {
        let coord = RefreshCoordinator::new();
        // Should not panic
        coord.complete("nonexistent").await;
        assert_eq!(coord.in_flight_count().await, 0);
    }

    #[tokio::test]
    async fn winner_after_complete_can_refresh_again() {
        let coord = RefreshCoordinator::new();

        // First round
        let first = coord.try_refresh("cred-1").await;
        assert!(matches!(first, RefreshAttempt::Winner(_)));
        coord.complete("cred-1").await;

        // Second round -- same credential can be refreshed again
        let second = coord.try_refresh("cred-1").await;
        assert!(matches!(second, RefreshAttempt::Winner(_)));
        coord.complete("cred-1").await;
    }

    #[tokio::test]
    async fn multiple_waiters_all_notified() {
        let coord = Arc::new(RefreshCoordinator::new());

        // Winner
        let _ = coord.try_refresh("cred-1").await;

        // Create 5 waiters with readiness signals
        let mut handles = Vec::new();
        let mut ready_rxs = Vec::new();
        for _ in 0..5 {
            let attempt = coord.try_refresh("cred-1").await;
            let notify = match attempt {
                RefreshAttempt::Waiter(n) => n,
                RefreshAttempt::Winner(_) => panic!("expected Waiter"),
            };
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
            ready_rxs.push(ready_rx);
            handles.push(tokio::spawn(async move {
                let notified = notify.notified();
                let _ = ready_tx.send(());
                notified.await;
                true
            }));
        }

        // Wait for all waiters to register
        for rx in ready_rxs {
            rx.await.unwrap();
        }

        // Complete -- all waiters should wake
        coord.complete("cred-1").await;

        for handle in handles {
            assert!(handle.await.unwrap());
        }
    }

    #[tokio::test]
    async fn default_creates_empty_coordinator() {
        let coord = RefreshCoordinator::default();
        assert_eq!(coord.in_flight_count().await, 0);
    }

    #[tokio::test]
    async fn circuit_breaker_opens_after_max_failures() {
        let coord = RefreshCoordinator::new();
        for _ in 0..5 {
            coord.record_failure("cred-1").await;
        }
        assert!(coord.is_circuit_open("cred-1").await);
    }

    #[tokio::test]
    async fn circuit_breaker_closed_initially() {
        let coord = RefreshCoordinator::new();
        assert!(!coord.is_circuit_open("cred-1").await);
    }

    #[tokio::test]
    async fn circuit_breaker_resets_on_success() {
        let coord = RefreshCoordinator::new();
        for _ in 0..5 {
            coord.record_failure("cred-1").await;
        }
        assert!(coord.is_circuit_open("cred-1").await);
        coord.record_success("cred-1").await;
        assert!(!coord.is_circuit_open("cred-1").await);
    }

    #[tokio::test]
    async fn circuit_breaker_below_threshold_stays_closed() {
        let coord = RefreshCoordinator::new();
        for _ in 0..4 {
            coord.record_failure("cred-1").await;
        }
        assert!(!coord.is_circuit_open("cred-1").await);
    }
}
