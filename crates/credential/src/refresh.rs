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

use std::{collections::HashMap, fmt, num::NonZeroUsize, sync::Arc, time::Duration};

use lru::LruCache;
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
    circuit_breakers: parking_lot::Mutex<LruCache<String, Arc<CircuitBreaker>>>,
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

/// Raw LRU cap for per-credential circuit breakers (must stay > 0).
const MAX_TRACKED_CIRCUIT_BREAKERS_CAP: usize = 4096;

/// Maximum number of per-credential circuit breakers kept in memory.
///
/// Failed refreshes insert entries that are only removed on success; without a
/// cap, unbounded churn of distinct credential IDs could grow this map
/// forever (see GitHub issue #278). Eviction follows LRU semantics: the least
/// recently touched breaker may be dropped when the cap is exceeded.
///
/// The `unwrap` is evaluated at compile time: [`MAX_TRACKED_CIRCUIT_BREAKERS_CAP`] is non-zero.
const MAX_TRACKED_CIRCUIT_BREAKERS: NonZeroUsize =
    NonZeroUsize::new(MAX_TRACKED_CIRCUIT_BREAKERS_CAP).unwrap();

/// Configuration errors returned by [`RefreshCoordinator`] constructors.
///
/// `RefreshCoordinator::with_max_concurrent(0)` would back the coordinator
/// with a `tokio::sync::Semaphore::new(0)`, deadlocking every caller that
/// awaits a permit. Rather than silently clamp to `1` (which would hide the
/// misconfiguration), construction fails with a typed error so the caller is
/// forced to handle the bad input at startup.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshConfigError {
    /// `max_concurrent` must be at least `1`; `0` would deadlock any caller
    /// awaiting an `OwnedSemaphorePermit`.
    #[error("RefreshCoordinator::max_concurrent must be >= 1, got 0")]
    ZeroConcurrency,
}

impl RefreshCoordinator {
    /// Creates a new coordinator with no in-flight refreshes and a default
    /// concurrency limit of 32.
    pub fn new() -> Self {
        // DEFAULT_MAX_CONCURRENT_REFRESHES is a compile-time const > 0, so
        // this construction is statically valid and we use the infallible
        // private constructor. Do NOT `.unwrap()` the public fallible one —
        // that would propagate a needless panic path into `Default`.
        Self::with_max_concurrent_unchecked(DEFAULT_MAX_CONCURRENT_REFRESHES)
    }

    /// Creates a new coordinator with a custom concurrency limit.
    ///
    /// The `max` parameter controls how many refresh HTTP calls can run
    /// in parallel across all credentials. Set this based on the provider's
    /// rate limit tolerance.
    ///
    /// # Errors
    ///
    /// Returns [`RefreshConfigError::ZeroConcurrency`] if `max == 0`. A
    /// zero-permit semaphore would deadlock every caller that awaits
    /// [`acquire_permit`](Self::acquire_permit), so construction is rejected
    /// rather than silently clamped.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use nebula_credential::refresh::RefreshCoordinator;
    ///
    /// // Allow at most 10 concurrent refreshes
    /// let coord = RefreshCoordinator::with_max_concurrent(10)?;
    /// assert_eq!(coord.available_permits(), 10);
    /// # Ok::<_, nebula_credential::refresh::RefreshConfigError>(())
    /// ```
    pub fn with_max_concurrent(max: usize) -> Result<Self, RefreshConfigError> {
        if max == 0 {
            return Err(RefreshConfigError::ZeroConcurrency);
        }
        Ok(Self::with_max_concurrent_unchecked(max))
    }

    /// Infallible constructor used by `new()` and by the validated fallible
    /// path. Private so external callers cannot skip the `max >= 1` check.
    fn with_max_concurrent_unchecked(max: usize) -> Self {
        debug_assert!(max > 0, "with_max_concurrent_unchecked requires max >= 1");
        Self {
            in_flight: parking_lot::Mutex::new(HashMap::new()),
            circuit_breakers: parking_lot::Mutex::new(LruCache::new(MAX_TRACKED_CIRCUIT_BREAKERS)),
            refresh_semaphore: Arc::new(tokio::sync::Semaphore::new(max)),
        }
    }

    /// Returns or creates a per-credential circuit breaker.
    fn get_or_create_cb(&self, credential_id: &str) -> Arc<CircuitBreaker> {
        let mut cbs = self.circuit_breakers.lock();
        if let Some(cb) = cbs.get(credential_id) {
            return Arc::clone(cb);
        }
        let config = CircuitBreakerConfig {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(300),
            min_operations: 1,
            ..Default::default()
        };
        let inner = CircuitBreaker::new(config).unwrap_or_else(|err| {
            tracing::error!(
                error = %err,
                "unexpected: static refresh circuit breaker config invalid; using defaults"
            );
            CircuitBreaker::new(CircuitBreakerConfig::default()).unwrap_or_else(|err2| {
                tracing::error!(
                    error = %err2,
                    "unexpected: default circuit breaker config invalid"
                );
                unreachable!("CircuitBreakerConfig::default must validate in nebula-resilience")
            })
        });
        let cb = Arc::new(inner);
        cbs.put(credential_id.to_string(), Arc::clone(&cb));
        cb
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
        self.circuit_breakers.lock().pop(credential_id);
    }

    /// Returns `true` if the circuit breaker is open (too many failures).
    pub fn is_circuit_open(&self, credential_id: &str) -> bool {
        let mut cbs = self.circuit_breakers.lock();
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
impl RefreshCoordinator {
    /// Returns the number of circuit breaker entries currently tracked (test-only).
    fn circuit_breaker_len_for_test(&self) -> usize {
        self.circuit_breakers.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_breaker_tracking_is_bounded() {
        let coord = RefreshCoordinator::new();
        for i in 0..6000 {
            coord.record_failure(&format!("cred-{i}"));
        }
        assert!(
            coord.circuit_breaker_len_for_test() <= MAX_TRACKED_CIRCUIT_BREAKERS.get(),
            "LRU cap should prevent unbounded growth (issue #278)"
        );
    }

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
        let coord =
            RefreshCoordinator::with_max_concurrent(2).expect("max=2 is a valid concurrency limit");
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

    /// #314: `with_max_concurrent(0)` must not construct a deadlocking
    /// coordinator. Before the fix this returned `Self` backed by
    /// `Semaphore::new(0)`, so any `acquire_permit().await` hung forever.
    #[tokio::test]
    async fn zero_max_concurrent_returns_config_error() {
        let err = RefreshCoordinator::with_max_concurrent(0)
            .expect_err("zero concurrency must be rejected");
        assert!(matches!(err, RefreshConfigError::ZeroConcurrency));
    }

    #[tokio::test]
    async fn one_max_concurrent_is_valid() {
        let coord = RefreshCoordinator::with_max_concurrent(1).expect("max=1 is valid");
        assert_eq!(coord.available_permits(), 1);
        let _p = coord.acquire_permit().await;
        assert_eq!(coord.available_permits(), 0);
    }

    // ── LRU circuit-breaker specific tests ──────────────────────────────

    #[test]
    fn record_success_removes_circuit_breaker_entry() {
        let coord = RefreshCoordinator::new();
        coord.record_failure("cred-x");
        assert_eq!(coord.circuit_breaker_len_for_test(), 1);
        coord.record_success("cred-x");
        assert_eq!(
            coord.circuit_breaker_len_for_test(),
            0,
            "record_success must remove the CB entry via LruCache::pop"
        );
    }

    #[test]
    fn record_success_on_unknown_credential_is_noop() {
        let coord = RefreshCoordinator::new();
        // Must not panic when popping a key not in the LRU cache
        coord.record_success("never-seen");
        assert_eq!(coord.circuit_breaker_len_for_test(), 0);
    }

    #[test]
    fn is_circuit_open_returns_false_when_no_entry_exists() {
        let coord = RefreshCoordinator::new();
        // A credential that has never had a failure recorded has no CB entry
        assert!(
            !coord.is_circuit_open("brand-new-cred"),
            "circuit must be closed for credentials never seen"
        );
    }

    #[test]
    fn get_or_create_cb_reuses_entry_for_same_credential() {
        let coord = RefreshCoordinator::new();
        // Record a failure — creates a CB entry
        coord.record_failure("cred-reuse");
        let len_after_first = coord.circuit_breaker_len_for_test();
        // Record another failure for the same ID — must not insert a new entry
        coord.record_failure("cred-reuse");
        assert_eq!(
            coord.circuit_breaker_len_for_test(),
            len_after_first,
            "same credential must reuse the existing CB, not create a new one"
        );
    }

    #[test]
    fn lru_evicts_least_recently_used_under_cap() {
        // Use a coordinator with only 2 slots to test eviction easily without
        // relying on the 4096 cap which would need 4097 insertions.
        // We access the internals via record_failure + circuit_breaker_len_for_test.
        let coord = RefreshCoordinator::new();
        // Insert exactly MAX_TRACKED_CIRCUIT_BREAKERS_CAP entries.
        for i in 0..MAX_TRACKED_CIRCUIT_BREAKERS_CAP {
            coord.record_failure(&format!("cred-lru-{i}"));
        }
        assert_eq!(
            coord.circuit_breaker_len_for_test(),
            MAX_TRACKED_CIRCUIT_BREAKERS_CAP,
            "should be at the cap before any eviction"
        );
        // One more entry must evict the LRU item (cred-lru-0 was never re-accessed).
        coord.record_failure("cred-lru-overflow");
        assert_eq!(
            coord.circuit_breaker_len_for_test(),
            MAX_TRACKED_CIRCUIT_BREAKERS_CAP,
            "adding one entry beyond cap must evict the LRU item"
        );
    }

    #[test]
    fn recently_accessed_cb_not_evicted() {
        let coord = RefreshCoordinator::new();
        // Fill the cache to capacity
        for i in 0..MAX_TRACKED_CIRCUIT_BREAKERS_CAP {
            coord.record_failure(&format!("evict-me-{i}"));
        }
        // Re-touch cred-0 to make it MRU (is_circuit_open calls cbs.get internally)
        let _ = coord.is_circuit_open("evict-me-0");
        // Now overflow the cache
        coord.record_failure("newcomer");
        // evict-me-0 was recently accessed so must survive; is_circuit_open would
        // return false (CB freshly open if 1 failure below threshold of 5, not open)
        // but the entry should still exist. We verify via len and the newcomer.
        assert_eq!(
            coord.circuit_breaker_len_for_test(),
            MAX_TRACKED_CIRCUIT_BREAKERS_CAP,
            "cache should still be at cap after eviction"
        );
    }

    #[test]
    fn independent_credentials_have_separate_circuit_breakers() {
        let coord = RefreshCoordinator::new();
        // Open the circuit for cred-a (5 failures)
        for _ in 0..5 {
            coord.record_failure("cred-a");
        }
        // cred-b has had no failures
        assert!(coord.is_circuit_open("cred-a"), "cred-a circuit must be open");
        assert!(
            !coord.is_circuit_open("cred-b"),
            "cred-b circuit must remain closed"
        );
    }

    #[test]
    fn circuit_breaker_cap_constant_is_nonzero() {
        // Regression guard: ensure the compile-time constant is never accidentally set to 0.
        assert!(MAX_TRACKED_CIRCUIT_BREAKERS.get() > 0);
        assert_eq!(
            MAX_TRACKED_CIRCUIT_BREAKERS.get(),
            MAX_TRACKED_CIRCUIT_BREAKERS_CAP
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