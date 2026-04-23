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
//! [`tokio::sync::oneshot::Receiver`] and wait until the winner completes.
//!
//! # Lost-wakeup safety
//!
//! Waiters register a `oneshot::Sender` into the in-flight entry **under
//! the same mutex** that the winner holds when inserting or removing the
//! entry. A winner cannot `complete()` past a waiter's registration — the
//! race window that existed with [`tokio::sync::Notify::notify_waiters`]
//! (where the waiter had to `enable()` its `Notified` future after the
//! lock was released) is closed by construction. See GitHub issue #268.
//!
//! The winner **must** call [`RefreshCoordinator::complete()`] when done
//! (success or failure) to wake waiters and remove the in-flight entry.
//! Wrapping the call in a `scopeguard` is recommended so the guarantee
//! survives panics and error paths.
//!
//! # Examples
//!
//! ```ignore
//! use nebula_engine::credential::refresh::{RefreshCoordinator, RefreshAttempt};
//!
//! let coord = RefreshCoordinator::new();
//!
//! match coord.try_refresh("cred-1") {
//!     RefreshAttempt::Winner => {
//!         // Perform refresh...
//!         coord.complete("cred-1");
//!     }
//!     RefreshAttempt::Waiter(rx) => {
//!         let _ = rx.await; // Err means the sender was dropped without sending
//!                           // (for example, coordinator shutdown or a bug/cancellation
//!                           // path that prevents `complete()` from running)
//!         // Re-read credential from store
//!     }
//! }
//! ```

use std::{collections::HashMap, fmt, num::NonZeroUsize, sync::Arc, time::Duration};

use lru::LruCache;
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome};
use tokio::sync::oneshot;

/// In-flight refresh entry.
///
/// Holds the pending `oneshot::Sender`s of every waiter registered for this
/// credential. Waiters push a sender under the coordinator's map lock; the
/// winner drains and fires every sender on completion.
struct InFlightEntry {
    /// Senders to wake on completion. `parking_lot::Mutex` because the list
    /// is touched only while holding (or having just released) the outer
    /// map lock, with no `.await` in between.
    senders: parking_lot::Mutex<Vec<oneshot::Sender<()>>>,
}

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
/// use nebula_engine::credential::refresh::{RefreshCoordinator, RefreshAttempt};
///
/// let coord = RefreshCoordinator::new();
///
/// // First caller wins
/// assert!(matches!(
///     coord.try_refresh("cred-1"),
///     RefreshAttempt::Winner,
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
    in_flight: parking_lot::Mutex<HashMap<String, Arc<InFlightEntry>>>,
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
///
/// Variants are intentionally **not** `#[non_exhaustive]` — the enum has
/// exactly two cases by construction (either you're the first caller or
/// you're not) and the caller must pattern-match both.
#[derive(Debug)]
pub enum RefreshAttempt {
    /// This caller won the race -- should perform the refresh, then call
    /// [`RefreshCoordinator::complete()`] to wake waiters and release the
    /// in-flight entry. Wrapping the call in a `scopeguard` is recommended
    /// so the guarantee survives panics and error paths.
    Winner,
    /// Another caller is already refreshing. Await the receiver; it
    /// resolves to `Ok(())` as soon as the winner calls
    /// [`RefreshCoordinator::complete()`].
    ///
    /// `Err(oneshot::error::RecvError)` means the sender was dropped
    /// **without being fired** — only reachable on abnormal paths
    /// (coordinator teardown, or a bug that removes the in-flight entry
    /// without draining its senders). It is *not* the signal for a normal
    /// winner completion. A winner that forgets `complete()` leaves the
    /// sender alive in the map: the receiver hangs until the caller-side
    /// timeout rather than erroring.
    ///
    /// No lost-wakeup race: the sender was pushed into the in-flight entry
    /// while holding the map mutex — the same mutex the winner holds when
    /// it removes the entry — so a waiter's registration and a winner's
    /// `complete()` are strictly ordered (GitHub issue #268).
    Waiter(oneshot::Receiver<()>),
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
    /// use nebula_engine::credential::refresh::RefreshCoordinator;
    ///
    /// // Allow at most 10 concurrent refreshes
    /// let coord = RefreshCoordinator::with_max_concurrent(10)?;
    /// assert_eq!(coord.available_permits(), 10);
    /// # Ok::<_, nebula_engine::credential::refresh::RefreshConfigError>(())
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
            reset_timeout: Duration::from_mins(5),
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
    /// (should perform the refresh), or [`RefreshAttempt::Waiter`] with a
    /// [`oneshot::Receiver`] if another refresh is already in progress.
    ///
    /// The waiter's sender is pushed into the in-flight entry while the
    /// map mutex is still held. This closes the lost-wakeup race present
    /// when the coordination primitive was `tokio::sync::Notify`: see
    /// GitHub issue #268 and the module-level docs.
    pub fn try_refresh(&self, credential_id: &str) -> RefreshAttempt {
        let mut map = self.in_flight.lock();
        if let Some(entry) = map.get(credential_id) {
            let (tx, rx) = oneshot::channel();
            entry.senders.lock().push(tx);
            RefreshAttempt::Waiter(rx)
        } else {
            let entry = Arc::new(InFlightEntry {
                senders: parking_lot::Mutex::new(Vec::new()),
            });
            map.insert(credential_id.to_string(), entry);
            RefreshAttempt::Winner
        }
    }

    /// Removes the in-flight entry for the given credential and wakes
    /// every waiter registered against it.
    ///
    /// Called by the winner after refresh completes (success or failure).
    /// Sync so it can be called from a `scopeguard` drop to prevent
    /// permanent in-flight map poisoning on panic.
    ///
    /// A `oneshot::Sender::send` that fails (receiver dropped by a
    /// cancelled waiter) is silently ignored — the cancelled task does
    /// not need to be woken.
    pub fn complete(&self, credential_id: &str) {
        let entry = self.in_flight.lock().remove(credential_id);
        if let Some(entry) = entry {
            let senders = std::mem::take(&mut *entry.senders.lock());
            for tx in senders {
                let _ = tx.send(());
            }
        }
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
        assert!(matches!(attempt, RefreshAttempt::Winner));
        assert_eq!(coord.in_flight_count(), 1);
    }

    #[tokio::test]
    async fn second_caller_waits() {
        let coord = RefreshCoordinator::new();
        let first = coord.try_refresh("cred-1");
        assert!(matches!(first, RefreshAttempt::Winner));

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
    async fn complete_wakes_waiters() {
        let coord = Arc::new(RefreshCoordinator::new());

        // Winner starts refresh
        assert!(matches!(
            coord.try_refresh("cred-1"),
            RefreshAttempt::Winner
        ));

        // Waiter gets the receiver
        let waiter_rx = match coord.try_refresh("cred-1") {
            RefreshAttempt::Waiter(rx) => rx,
            RefreshAttempt::Winner => panic!("expected Waiter"),
        };

        let waiter = tokio::spawn(async move { waiter_rx.await.is_ok() });

        // Winner completes: remove entry + drain senders
        coord.complete("cred-1");

        // Waiter observes the completion
        let result = waiter.await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn independent_credentials_do_not_interfere() {
        let coord = RefreshCoordinator::new();

        let a = coord.try_refresh("cred-a");
        let b = coord.try_refresh("cred-b");

        // Both should win -- different credentials
        assert!(matches!(a, RefreshAttempt::Winner));
        assert!(matches!(b, RefreshAttempt::Winner));
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
        assert!(matches!(first, RefreshAttempt::Winner));
        coord.complete("cred-1");

        // Second round -- same credential can be refreshed again
        let second = coord.try_refresh("cred-1");
        assert!(matches!(second, RefreshAttempt::Winner));
        coord.complete("cred-1");
    }

    #[tokio::test]
    async fn multiple_waiters_all_notified() {
        let coord = Arc::new(RefreshCoordinator::new());

        // Winner
        assert!(matches!(
            coord.try_refresh("cred-1"),
            RefreshAttempt::Winner
        ));

        // Create 5 waiters; each just awaits its receiver.
        let mut handles = Vec::new();
        for _ in 0..5 {
            let rx = match coord.try_refresh("cred-1") {
                RefreshAttempt::Waiter(rx) => rx,
                RefreshAttempt::Winner => panic!("expected Waiter"),
            };
            handles.push(tokio::spawn(async move { rx.await.is_ok() }));
        }

        // Complete drains every sender
        coord.complete("cred-1");

        for handle in handles {
            assert!(handle.await.unwrap());
        }
    }

    /// Regression for GitHub issue #268: pushing the waiter's sender into
    /// the in-flight entry happens under the same map mutex the winner
    /// acquires in `complete()`, so the winner cannot race past a waiter's
    /// registration. Exercises the tight timing that used to lose wakeups
    /// under `Notify::notify_waiters()` (where the waiter had to
    /// `enable()` its `Notified` future after the lock was released).
    #[tokio::test]
    async fn waiter_registered_under_lock_is_never_missed() {
        let coord = Arc::new(RefreshCoordinator::new());

        // Winner takes the entry.
        assert!(matches!(
            coord.try_refresh("cred-1"),
            RefreshAttempt::Winner
        ));

        // Register a waiter then immediately complete — with the old
        // Notify-based design, a post-return `enable()` could miss the
        // wakeup fired here. With oneshot-under-lock, the sender is
        // already in the entry's Vec when `complete()` drains it.
        let rx = match coord.try_refresh("cred-1") {
            RefreshAttempt::Waiter(rx) => rx,
            RefreshAttempt::Winner => panic!("expected Waiter"),
        };
        coord.complete("cred-1");

        tokio::time::timeout(Duration::from_millis(50), rx)
            .await
            .expect("waiter must observe completion even when it fires immediately")
            .expect("sender must be fired, not dropped");
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
        assert!(matches!(attempt, RefreshAttempt::Winner));
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
        assert!(matches!(attempt2, RefreshAttempt::Winner));
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
        assert_eq!(coord.available_permits(), DEFAULT_MAX_CONCURRENT_REFRESHES);
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

    /// Regression guard: a waiter that drops its receiver (cancellation)
    /// must not prevent the winner from completing — `oneshot::Sender::send`
    /// on a dropped receiver returns `Err(_)` which we silently ignore.
    #[tokio::test]
    async fn cancelled_waiter_does_not_stall_winner() {
        let coord = Arc::new(RefreshCoordinator::new());
        assert!(matches!(
            coord.try_refresh("cred-1"),
            RefreshAttempt::Winner
        ));

        // Register a waiter and then immediately drop the receiver
        // (simulates the waiting task being cancelled before the winner
        // completes). `complete()` must still run cleanly.
        match coord.try_refresh("cred-1") {
            RefreshAttempt::Waiter(rx) => drop(rx),
            RefreshAttempt::Winner => panic!("expected Waiter"),
        }

        coord.complete("cred-1");
        assert_eq!(coord.in_flight_count(), 0);
    }
}
