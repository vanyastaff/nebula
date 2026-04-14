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
//! [`Notify`](tokio::sync::Notify) handle and wait until the winner completes.
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
//! match coord.try_refresh("cred-1") {
//!     RefreshAttempt::Winner(_notify) => {
//!         // Perform refresh...
//!         coord.complete("cred-1");
//!     }
//!     RefreshAttempt::Waiter(notify) => {
//!         notify.notified().await;
//!         // Re-read credential from store
//!     }
//! }
//! ```

use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome};
use tokio::sync::Notify;

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
///     coord.try_refresh("cred-1"),
///     RefreshAttempt::Winner(_),
/// ));
///
/// // Second caller waits
/// assert!(matches!(
///     coord.try_refresh("cred-1"),
///     RefreshAttempt::Waiter(_),
/// ));
///
/// coord.complete("cred-1");
/// ```
pub struct RefreshCoordinator {
    /// In-flight refresh entries. Uses `parking_lot::Mutex` (not `tokio::sync::Mutex`)
    /// because the lock is held only for brief `HashMap` insert/remove/get operations
    /// with no `.await` while locked. This allows `complete()` to be sync, which is
    /// critical for calling it from a `scopeguard` drop (B8 fix).
    in_flight: parking_lot::Mutex<HashMap<String, Arc<Notify>>>,
    /// Per-credential circuit breakers via `nebula-resilience`.
    circuit_breakers: parking_lot::Mutex<HashMap<String, Arc<CircuitBreaker>>>,
    /// Global concurrency limiter for refresh operations.
    ///
    /// Prevents cascading failures when many credentials expire simultaneously
    /// and the provider rate-limits (429). Default: 32 concurrent refreshes.
    refresh_semaphore: Arc<tokio::sync::Semaphore>,
}

impl fmt::Debug for RefreshCoordinator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let breaker_count = self.circuit_breakers.lock().len();
        f.debug_struct("RefreshCoordinator")
            .field("circuit_breakers", &breaker_count)
            .field(
                "available_permits",
                &self.refresh_semaphore.available_permits(),
            )
            .finish_non_exhaustive()
    }
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

/// Default maximum number of concurrent refresh operations.
const DEFAULT_MAX_CONCURRENT_REFRESHES: usize = 32;

impl RefreshCoordinator {
    /// Creates a new coordinator with no in-flight refreshes and a default
    /// concurrency limit of 32.
    pub fn new() -> Self {
        Self::with_max_concurrent(DEFAULT_MAX_CONCURRENT_REFRESHES)
    }

    /// Creates a new coordinator with a custom concurrency limit.
    ///
    /// The `max` parameter controls how many refresh HTTP calls can run
    /// in parallel across all credentials. Set this based on the provider's
    /// rate limit tolerance.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use nebula_credential::refresh::RefreshCoordinator;
    ///
    /// // Allow at most 10 concurrent refreshes
    /// let coord = RefreshCoordinator::with_max_concurrent(10);
    /// assert_eq!(coord.available_permits(), 10);
    /// ```
    #[must_use]
    pub fn with_max_concurrent(max: usize) -> Self {
        Self {
            in_flight: parking_lot::Mutex::new(HashMap::new()),
            circuit_breakers: parking_lot::Mutex::new(HashMap::new()),
            refresh_semaphore: Arc::new(tokio::sync::Semaphore::new(max)),
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
    pub fn try_refresh(&self, credential_id: &str) -> RefreshAttempt {
        let mut map = self.in_flight.lock();
        if let Some(notify) = map.get(credential_id) {
            RefreshAttempt::Waiter(Arc::clone(notify))
        } else {
            let notify = Arc::new(Notify::new());
            map.insert(credential_id.to_string(), Arc::clone(&notify));
            RefreshAttempt::Winner(notify)
        }
    }

    /// Removes the in-flight entry for the given credential.
    ///
    /// Called by the winner after refresh completes (success or failure).
    /// Sync so it can be called from a `scopeguard` drop to prevent
    /// permanent in-flight map poisoning on panic.
    pub fn complete(&self, credential_id: &str) {
        self.in_flight.lock().remove(credential_id);
    }

    /// Returns the number of credentials currently being refreshed.
    ///
    /// Primarily useful for testing and diagnostics.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.lock().len()
    }

    /// Acquires a permit from the global refresh concurrency limiter.
    ///
    /// The returned permit must be held for the entire duration of the
    /// refresh HTTP call. It is released automatically on drop (including
    /// when a `scopeguard` fires on panic or timeout).
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe: dropping the future before completion
    /// does not consume a permit.
    pub async fn acquire_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.refresh_semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("refresh semaphore closed unexpectedly")
    }

    /// Returns the number of available permits in the concurrency limiter.
    ///
    /// Primarily useful for testing and diagnostics.
    pub fn available_permits(&self) -> usize {
        self.refresh_semaphore.available_permits()
    }

    /// Records a refresh failure for circuit breaker tracking.
    pub fn record_failure(&self, credential_id: &str) {
        self.get_or_create_cb(credential_id)
            .record_outcome(Outcome::Failure);
    }

    /// Records a successful refresh, resetting the circuit breaker.
    pub fn record_success(&self, credential_id: &str) {
        // Remove the CB entirely on success — full reset.
        self.circuit_breakers.lock().remove(credential_id);
    }

    /// Returns `true` if the circuit breaker is open (too many failures).
    pub fn is_circuit_open(&self, credential_id: &str) -> bool {
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
        let attempt = coord.try_refresh("cred-1");
        assert!(matches!(attempt, RefreshAttempt::Winner(_)));
        assert_eq!(coord.in_flight_count(), 1);
    }

    #[tokio::test]
    async fn second_caller_waits() {
        let coord = RefreshCoordinator::new();
        let first = coord.try_refresh("cred-1");
        assert!(matches!(first, RefreshAttempt::Winner(_)));

        let second = coord.try_refresh("cred-1");
        assert!(matches!(second, RefreshAttempt::Waiter(_)));
    }

    #[tokio::test]
    async fn complete_removes_in_flight_entry() {
        let coord = RefreshCoordinator::new();
        let _ = coord.try_refresh("cred-1");
        assert_eq!(coord.in_flight_count(), 1);

        coord.complete("cred-1");
        assert_eq!(coord.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn complete_and_notify_wakes_waiters() {
        let coord = Arc::new(RefreshCoordinator::new());

        // Winner starts refresh -- extract the Notify handle
        let winner_notify = match coord.try_refresh("cred-1") {
            RefreshAttempt::Winner(n) => n,
            RefreshAttempt::Waiter(_) => panic!("expected Winner"),
        };

        // Waiter gets Notify
        let waiter_notify = match coord.try_refresh("cred-1") {
            RefreshAttempt::Waiter(n) => n,
            RefreshAttempt::Winner(_) => panic!("expected Waiter"),
        };

        // Use a oneshot to signal the waiter task is ready
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

        let waiter = tokio::spawn(async move {
            let notified = waiter_notify.notified();
            // Signal that we've registered interest
            let _ = ready_tx.send(());
            notified.await;
            true
        });

        // Wait for waiter to register
        ready_rx.await.unwrap();

        // Winner completes: remove entry + notify (as scopeguard does in resolver)
        coord.complete("cred-1");
        winner_notify.notify_waiters();

        // Waiter should have been woken
        let result = waiter.await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn independent_credentials_do_not_interfere() {
        let coord = RefreshCoordinator::new();

        let a = coord.try_refresh("cred-a");
        let b = coord.try_refresh("cred-b");

        // Both should win -- different credentials
        assert!(matches!(a, RefreshAttempt::Winner(_)));
        assert!(matches!(b, RefreshAttempt::Winner(_)));
        assert_eq!(coord.in_flight_count(), 2);

        coord.complete("cred-a");
        assert_eq!(coord.in_flight_count(), 1);

        coord.complete("cred-b");
        assert_eq!(coord.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn complete_on_unknown_credential_is_noop() {
        let coord = RefreshCoordinator::new();
        // Should not panic
        coord.complete("nonexistent");
        assert_eq!(coord.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn winner_after_complete_can_refresh_again() {
        let coord = RefreshCoordinator::new();

        // First round
        let first = coord.try_refresh("cred-1");
        assert!(matches!(first, RefreshAttempt::Winner(_)));
        coord.complete("cred-1");

        // Second round -- same credential can be refreshed again
        let second = coord.try_refresh("cred-1");
        assert!(matches!(second, RefreshAttempt::Winner(_)));
        coord.complete("cred-1");
    }

    #[tokio::test]
    async fn multiple_waiters_all_notified() {
        let coord = Arc::new(RefreshCoordinator::new());

        // Winner -- extract Notify handle
        let winner_notify = match coord.try_refresh("cred-1") {
            RefreshAttempt::Winner(n) => n,
            RefreshAttempt::Waiter(_) => panic!("expected Winner"),
        };

        // Create 5 waiters with readiness signals
        let mut handles = Vec::new();
        let mut ready_rxs = Vec::new();
        for _ in 0..5 {
            let attempt = coord.try_refresh("cred-1");
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

        // Complete + notify -- all waiters should wake
        coord.complete("cred-1");
        winner_notify.notify_waiters();

        for handle in handles {
            assert!(handle.await.unwrap());
        }
    }

    #[tokio::test]
    async fn default_creates_empty_coordinator() {
        let coord = RefreshCoordinator::default();
        assert_eq!(coord.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn circuit_breaker_opens_after_max_failures() {
        let coord = RefreshCoordinator::new();
        for _ in 0..5 {
            coord.record_failure("cred-1");
        }
        assert!(coord.is_circuit_open("cred-1"));
    }

    #[tokio::test]
    async fn circuit_breaker_closed_initially() {
        let coord = RefreshCoordinator::new();
        assert!(!coord.is_circuit_open("cred-1"));
    }

    #[tokio::test]
    async fn circuit_breaker_resets_on_success() {
        let coord = RefreshCoordinator::new();
        for _ in 0..5 {
            coord.record_failure("cred-1");
        }
        assert!(coord.is_circuit_open("cred-1"));
        coord.record_success("cred-1");
        assert!(!coord.is_circuit_open("cred-1"));
    }

    #[tokio::test]
    async fn circuit_breaker_below_threshold_stays_closed() {
        let coord = RefreshCoordinator::new();
        for _ in 0..4 {
            coord.record_failure("cred-1");
        }
        assert!(!coord.is_circuit_open("cred-1"));
    }

    /// B8: Verifies that dropping a Winner without calling complete()
    /// leaves the in-flight entry, and that a subsequent sync complete()
    /// (as scopeguard would call) cleans it up so new callers become Winners.
    #[tokio::test]
    async fn inflight_entry_cleaned_after_sync_complete() {
        let coord = RefreshCoordinator::new();

        // Get Winner
        let attempt = coord.try_refresh("cred-1");
        assert!(matches!(attempt, RefreshAttempt::Winner(_)));
        assert_eq!(coord.in_flight_count(), 1);

        // Drop without calling complete -- simulates panic before scopeguard
        drop(attempt);

        // Entry is still in-flight (poisoned state without scopeguard)
        assert_eq!(coord.in_flight_count(), 1);

        // Manually call complete (as scopeguard would on drop)
        coord.complete("cred-1");
        assert_eq!(coord.in_flight_count(), 0);

        // Next call should get Winner (entry was cleaned)
        let attempt2 = coord.try_refresh("cred-1");
        assert!(matches!(attempt2, RefreshAttempt::Winner(_)));
        coord.complete("cred-1");
    }

    #[tokio::test]
    async fn semaphore_limits_concurrent_refreshes() {
        let coord = RefreshCoordinator::with_max_concurrent(2);
        assert_eq!(coord.available_permits(), 2);

        let _p1 = coord.acquire_permit().await;
        assert_eq!(coord.available_permits(), 1);

        let _p2 = coord.acquire_permit().await;
        assert_eq!(coord.available_permits(), 0);

        // Third acquire would block -- just verify count is zero
        drop(_p1);
        assert_eq!(coord.available_permits(), 1);

        drop(_p2);
        assert_eq!(coord.available_permits(), 2);
    }

    #[tokio::test]
    async fn default_coordinator_has_default_permits() {
        let coord = RefreshCoordinator::new();
        assert_eq!(
            coord.available_permits(),
            super::DEFAULT_MAX_CONCURRENT_REFRESHES
        );
    }

    /// Pins the invariant the resolver's Waiter path relies on:
    /// `Notified::enable()` registers the waiter *before* the first poll,
    /// so a subsequent `notify_waiters()` will wake it.
    ///
    /// Without `enable()`, this is a classic lost-wakeup race: a winner
    /// that calls `notify_waiters()` between `notify.notified()` returning
    /// and the waiter polling it will find an empty waiter queue and leave
    /// the waiter stalled. In the resolver that used to mean a 60-second
    /// stall per lost wakeup before the post-wait store re-read could
    /// rescue latency.
    #[tokio::test]
    async fn pre_enabled_notified_receives_notify_waiters() {
        let notify = Arc::new(Notify::new());

        // Create + pre-enable BEFORE the notify_waiters() call. This is
        // the exact pattern in `resolver.rs::resolve_and_refresh`.
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        // Now fire notify_waiters *before* we poll the future for the
        // first time. With enable() this is fine; without it this test
        // would hang until timeout.
        notify.notify_waiters();

        // Pre-enabled future must complete effectively immediately.
        tokio::time::timeout(Duration::from_millis(50), notified)
            .await
            .expect("pre-enabled Notified must wake after notify_waiters");
    }
}
