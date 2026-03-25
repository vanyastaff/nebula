//! CAS-based state machine preventing thundering herd on dead backends.
//!
//! [`RecoveryGate`] serializes recovery attempts so that only one caller
//! at a time performs the expensive probe. Other callers either wait for
//! notification or receive an error with a retry-at hint.
//!
//! # State machine
//!
//! ```text
//! Idle ──try_begin──▶ InProgress ──resolve──▶ Idle
//!  ▲                       │
//!  │                  fail_transient
//!  │                       │
//!  │                       ▼
//!  └──(retry_at expired)── Failed ──fail_permanent──▶ PermanentlyFailed
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use tokio::sync::Notify;

/// Maximum backoff cap (5 minutes).
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// Current state of a [`RecoveryGate`].
#[derive(Debug, Clone)]
pub enum GateState {
    /// No recovery in progress; the backend is presumed healthy or untested.
    Idle,
    /// A recovery attempt is underway.
    InProgress {
        /// Which attempt number this is (1-based).
        attempt: u32,
    },
    /// The last attempt failed transiently; retry allowed after `retry_at`.
    Failed {
        /// Human-readable failure reason.
        message: String,
        /// Earliest instant a new attempt may begin.
        retry_at: Instant,
        /// How many attempts have been made so far.
        attempt: u32,
    },
    /// The backend is permanently broken; no further recovery attempts.
    PermanentlyFailed {
        /// Human-readable failure reason.
        message: String,
    },
}

/// Configuration for a [`RecoveryGate`].
#[derive(Debug, Clone)]
pub struct RecoveryGateConfig {
    /// Maximum recovery attempts before requiring manual reset.
    pub max_attempts: u32,
    /// Base backoff duration (doubled each attempt).
    pub base_backoff: Duration,
}

impl Default for RecoveryGateConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_backoff: Duration::from_secs(1),
        }
    }
}

/// Outcome of [`RecoveryGate::try_begin`] when the caller cannot proceed.
#[derive(Debug)]
pub enum TryBeginError {
    /// Another caller is already recovering; wait on the [`RecoveryWaiter`].
    AlreadyInProgress(RecoveryWaiter),
    /// The gate is in [`GateState::Failed`] and the backoff has not expired.
    RetryLater {
        /// When the next attempt is allowed.
        retry_at: Instant,
    },
    /// The gate is permanently failed — no recovery possible without reset.
    PermanentlyFailed {
        /// The failure message.
        message: String,
    },
}

/// RAII ticket granting exclusive recovery rights.
///
/// If dropped without calling [`resolve`](RecoveryTicket::resolve) or
/// [`fail_transient`](RecoveryTicket::fail_transient) /
/// [`fail_permanent`](RecoveryTicket::fail_permanent), the gate
/// automatically transitions to [`GateState::Failed`] with a backoff.
pub struct RecoveryTicket {
    gate: Arc<RecoveryGateInner>,
    attempt: u32,
    consumed: bool,
}

impl std::fmt::Debug for RecoveryTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryTicket")
            .field("attempt", &self.attempt)
            .field("consumed", &self.consumed)
            .finish()
    }
}

impl RecoveryTicket {
    /// Marks the recovery as successful, returning the gate to
    /// [`GateState::Idle`] and waking all waiters.
    pub fn resolve(mut self) {
        self.consumed = true;
        self.gate.state.store(Arc::new(GateState::Idle));
        self.gate.notify.notify_waiters();
    }

    /// Marks the recovery as transiently failed with exponential backoff.
    pub fn fail_transient(mut self, message: impl Into<String>) {
        self.consumed = true;
        let backoff = compute_backoff(self.gate.base_backoff, self.attempt);
        let state = GateState::Failed {
            message: message.into(),
            retry_at: Instant::now() + backoff,
            attempt: self.attempt,
        };
        self.gate.state.store(Arc::new(state));
        self.gate.notify.notify_waiters();
    }

    /// Marks the recovery as permanently failed — no further attempts.
    pub fn fail_permanent(mut self, message: impl Into<String>) {
        self.consumed = true;
        let state = GateState::PermanentlyFailed {
            message: message.into(),
        };
        self.gate.state.store(Arc::new(state));
        self.gate.notify.notify_waiters();
    }

    /// Returns the current attempt number (1-based).
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

impl Drop for RecoveryTicket {
    fn drop(&mut self) {
        if !self.consumed {
            let backoff = compute_backoff(self.gate.base_backoff, self.attempt);
            let state = GateState::Failed {
                message: "recovery ticket dropped without resolution".into(),
                retry_at: Instant::now() + backoff,
                attempt: self.attempt,
            };
            self.gate.state.store(Arc::new(state));
            self.gate.notify.notify_waiters();
        }
    }
}

/// Handle returned when another caller already holds the recovery ticket.
///
/// Call [`wait`](RecoveryWaiter::wait) to block until the gate changes state.
#[derive(Debug)]
pub struct RecoveryWaiter {
    gate: Arc<RecoveryGateInner>,
}

impl RecoveryWaiter {
    /// Waits for the gate to leave [`GateState::InProgress`], then returns
    /// the new state.
    pub async fn wait(&self) -> GateState {
        loop {
            self.gate.notify.notified().await;
            let snap = self.gate.state.load();
            if !matches!(**snap, GateState::InProgress { .. }) {
                return (**snap).clone();
            }
        }
    }
}

/// Shared interior of [`RecoveryGate`].
struct RecoveryGateInner {
    state: ArcSwap<GateState>,
    notify: Notify,
    max_attempts: u32,
    base_backoff: Duration,
}

impl std::fmt::Debug for RecoveryGateInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryGateInner")
            .field("state", &*self.state.load())
            .field("max_attempts", &self.max_attempts)
            .field("base_backoff", &self.base_backoff)
            .finish()
    }
}

/// CAS-based recovery gate preventing thundering herd.
///
/// Only one caller at a time can perform recovery; others wait or receive
/// a retry-at hint.
///
/// # Examples
///
/// ```
/// use nebula_resource::recovery::{RecoveryGate, RecoveryGateConfig};
///
/// let gate = RecoveryGate::new(RecoveryGateConfig::default());
///
/// // First caller wins the ticket
/// match gate.try_begin() {
///     Ok(ticket) => ticket.resolve(),
///     Err(_) => unreachable!("gate starts idle"),
/// }
/// ```
#[derive(Debug, Clone)]
pub struct RecoveryGate {
    inner: Arc<RecoveryGateInner>,
}

impl RecoveryGate {
    /// Creates a new gate in [`GateState::Idle`].
    pub fn new(config: RecoveryGateConfig) -> Self {
        Self {
            inner: Arc::new(RecoveryGateInner {
                state: ArcSwap::from_pointee(GateState::Idle),
                notify: Notify::new(),
                max_attempts: config.max_attempts,
                base_backoff: config.base_backoff,
            }),
        }
    }

    /// Attempts to begin a recovery.
    ///
    /// # Errors
    ///
    /// Returns [`TryBeginError::AlreadyInProgress`] with a waiter if another
    /// caller already holds the ticket, [`TryBeginError::RetryLater`] if the
    /// backoff has not expired, or [`TryBeginError::PermanentlyFailed`] if
    /// the gate is permanently failed.
    pub fn try_begin(&self) -> Result<RecoveryTicket, TryBeginError> {
        loop {
            let current = self.inner.state.load();
            match &**current {
                GateState::Idle => {
                    let next = Arc::new(GateState::InProgress { attempt: 1 });
                    let prev = self.inner.state.compare_and_swap(&current, next);
                    if Arc::ptr_eq(&prev, &current) {
                        return Ok(RecoveryTicket {
                            gate: Arc::clone(&self.inner),
                            attempt: 1,
                            consumed: false,
                        });
                    }
                    // CAS failed — retry the loop.
                }
                GateState::InProgress { .. } => {
                    return Err(TryBeginError::AlreadyInProgress(RecoveryWaiter {
                        gate: Arc::clone(&self.inner),
                    }));
                }
                GateState::Failed {
                    retry_at, attempt, ..
                } => {
                    let now = Instant::now();
                    if now < *retry_at {
                        return Err(TryBeginError::RetryLater {
                            retry_at: *retry_at,
                        });
                    }
                    let next_attempt = attempt + 1;
                    if next_attempt > self.inner.max_attempts {
                        // Exceeded max attempts — transition to permanent.
                        let next = Arc::new(GateState::PermanentlyFailed {
                            message: format!(
                                "max recovery attempts ({}) exceeded",
                                self.inner.max_attempts
                            ),
                        });
                        self.inner.state.store(next);
                        self.inner.notify.notify_waiters();
                        return Err(TryBeginError::PermanentlyFailed {
                            message: format!(
                                "max recovery attempts ({}) exceeded",
                                self.inner.max_attempts
                            ),
                        });
                    }
                    let next = Arc::new(GateState::InProgress {
                        attempt: next_attempt,
                    });
                    let prev = self.inner.state.compare_and_swap(&current, next);
                    if Arc::ptr_eq(&prev, &current) {
                        return Ok(RecoveryTicket {
                            gate: Arc::clone(&self.inner),
                            attempt: next_attempt,
                            consumed: false,
                        });
                    }
                    // CAS failed — retry the loop.
                }
                GateState::PermanentlyFailed { message } => {
                    return Err(TryBeginError::PermanentlyFailed {
                        message: message.clone(),
                    });
                }
            }
        }
    }

    /// Returns a snapshot of the current gate state.
    pub fn state(&self) -> GateState {
        (**self.inner.state.load()).clone()
    }

    /// Forces the gate back to [`GateState::Idle`] (admin override).
    pub fn reset(&self) {
        self.inner.state.store(Arc::new(GateState::Idle));
        self.inner.notify.notify_waiters();
    }
}

/// Computes exponential backoff: `base * 2^(attempt - 1)`, capped at 5 min.
fn compute_backoff(base: Duration, attempt: u32) -> Duration {
    let multiplier = 1u64
        .checked_shl(attempt.saturating_sub(1))
        .unwrap_or(u64::MAX);
    let backoff = base.saturating_mul(multiplier as u32);
    if backoff > MAX_BACKOFF {
        MAX_BACKOFF
    } else {
        backoff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_gate() -> RecoveryGate {
        RecoveryGate::new(RecoveryGateConfig::default())
    }

    #[test]
    fn idle_gate_grants_ticket() {
        let gate = default_gate();
        let ticket = gate.try_begin().expect("should grant ticket");
        assert_eq!(ticket.attempt(), 1);
        ticket.resolve();
    }

    #[test]
    fn second_caller_gets_waiter() {
        let gate = default_gate();
        let _ticket = gate.try_begin().expect("first caller wins");
        match gate.try_begin() {
            Err(TryBeginError::AlreadyInProgress(_)) => {} // expected
            other => panic!("expected AlreadyInProgress, got: {other:?}"),
        }
    }

    #[test]
    fn resolve_returns_to_idle() {
        let gate = default_gate();
        let ticket = gate.try_begin().unwrap();
        ticket.resolve();
        assert!(matches!(gate.state(), GateState::Idle));
    }

    #[test]
    fn fail_transient_sets_failed_state() {
        let gate = default_gate();
        let ticket = gate.try_begin().unwrap();
        ticket.fail_transient("connection refused");
        match gate.state() {
            GateState::Failed {
                message, attempt, ..
            } => {
                assert_eq!(message, "connection refused");
                assert_eq!(attempt, 1);
            }
            other => panic!("expected Failed, got: {other:?}"),
        }
    }

    #[test]
    fn fail_permanent_blocks_further_attempts() {
        let gate = default_gate();
        let ticket = gate.try_begin().unwrap();
        ticket.fail_permanent("certificate expired");
        match gate.try_begin() {
            Err(TryBeginError::PermanentlyFailed { message }) => {
                assert_eq!(message, "certificate expired");
            }
            other => panic!("expected PermanentlyFailed, got: {other:?}"),
        }
    }

    #[test]
    fn drop_without_resolve_auto_fails() {
        let gate = default_gate();
        {
            let _ticket = gate.try_begin().unwrap();
            // Dropped here without resolve/fail.
        }
        assert!(matches!(gate.state(), GateState::Failed { .. }));
    }

    #[test]
    fn reset_clears_permanent_failure() {
        let gate = default_gate();
        let ticket = gate.try_begin().unwrap();
        ticket.fail_permanent("dead");
        assert!(matches!(gate.state(), GateState::PermanentlyFailed { .. }));
        gate.reset();
        assert!(matches!(gate.state(), GateState::Idle));
    }

    #[test]
    fn backoff_escalates_exponentially() {
        let base = Duration::from_millis(100);
        assert_eq!(compute_backoff(base, 1), Duration::from_millis(100));
        assert_eq!(compute_backoff(base, 2), Duration::from_millis(200));
        assert_eq!(compute_backoff(base, 3), Duration::from_millis(400));
        assert_eq!(compute_backoff(base, 4), Duration::from_millis(800));
    }

    #[test]
    fn backoff_caps_at_five_minutes() {
        let base = Duration::from_secs(60);
        // 60 * 2^4 = 960s > 300s cap
        assert_eq!(compute_backoff(base, 5), MAX_BACKOFF);
    }

    #[test]
    fn retry_after_expired_allows_new_attempt() {
        let config = RecoveryGateConfig {
            max_attempts: 5,
            base_backoff: Duration::from_millis(0), // zero backoff for test
        };
        let gate = RecoveryGate::new(config);

        let ticket = gate.try_begin().unwrap();
        ticket.fail_transient("timeout");

        // Backoff is 0ms, so retry_at is already in the past.
        let ticket2 = gate.try_begin().expect("should allow retry after expiry");
        assert_eq!(ticket2.attempt(), 2);
        ticket2.resolve();
    }

    #[test]
    fn max_attempts_triggers_permanent_failure() {
        let config = RecoveryGateConfig {
            max_attempts: 2,
            base_backoff: Duration::from_millis(0),
        };
        let gate = RecoveryGate::new(config);

        // Attempt 1
        let t1 = gate.try_begin().unwrap();
        t1.fail_transient("fail 1");

        // Attempt 2
        let t2 = gate.try_begin().unwrap();
        t2.fail_transient("fail 2");

        // Attempt 3 exceeds max_attempts=2 → permanent
        match gate.try_begin() {
            Err(TryBeginError::PermanentlyFailed { message }) => {
                assert!(message.contains("exceeded"), "msg: {message}");
            }
            other => panic!("expected PermanentlyFailed, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn waiter_unblocks_on_resolve() {
        let gate = default_gate();
        let ticket = gate.try_begin().unwrap();

        let gate2 = gate.clone();
        let handle = tokio::spawn(async move {
            match gate2.try_begin() {
                Err(TryBeginError::AlreadyInProgress(waiter)) => waiter.wait().await,
                other => panic!("expected AlreadyInProgress, got: {other:?}"),
            }
        });

        // Give the spawned task a moment to reach the wait point.
        tokio::task::yield_now().await;
        ticket.resolve();

        let state = handle.await.unwrap();
        assert!(matches!(state, GateState::Idle));
    }

    #[tokio::test]
    async fn concurrent_try_begin_only_one_wins() {
        let gate = default_gate();
        let mut winners = 0u32;
        let mut waiters = 0u32;

        for _ in 0..10 {
            match gate.try_begin() {
                Ok(ticket) => {
                    winners += 1;
                    ticket.resolve();
                }
                Err(TryBeginError::AlreadyInProgress(_)) => {
                    waiters += 1;
                }
                Err(other) => panic!("unexpected: {other:?}"),
            }
        }

        // All calls are sequential in this test, so each one should win
        // after the previous resolves.
        assert_eq!(winners, 10);
        assert_eq!(waiters, 0);
    }

    #[test]
    fn fail_transient_then_resolve_cycle() {
        let config = RecoveryGateConfig {
            max_attempts: 10,
            base_backoff: Duration::from_millis(0),
        };
        let gate = RecoveryGate::new(config);

        // Fail twice, then succeed.
        let t = gate.try_begin().unwrap();
        t.fail_transient("fail 1");

        let t = gate.try_begin().unwrap();
        t.fail_transient("fail 2");

        let t = gate.try_begin().unwrap();
        assert_eq!(t.attempt(), 3);
        t.resolve();

        assert!(matches!(gate.state(), GateState::Idle));
    }
}
