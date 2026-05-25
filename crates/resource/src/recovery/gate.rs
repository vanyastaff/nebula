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

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use arc_swap::ArcSwap;
use nebula_core::ResourceKey;
use tokio::sync::{Notify, broadcast};

use crate::events::ResourceEvent;

/// Maximum backoff cap (5 minutes).
const MAX_BACKOFF: Duration = Duration::from_mins(5);

/// Wiring that lets a gate emit [`ResourceEvent::RecoveryGateChanged`] on
/// every state transition. Attached once by the manager at registration time
/// (see [`RecoveryGate::set_event_sink`]).
#[derive(Debug)]
struct EventSink {
    tx: broadcast::Sender<ResourceEvent>,
    key: ResourceKey,
}

/// Returns a short, redacted label for a gate state — used as the `state`
/// field of [`ResourceEvent::RecoveryGateChanged`].
///
/// **Redaction contract:** the label intentionally omits the human-readable
/// failure message (it can echo upstream service text); only the variant
/// shape + attempt counter cross the event boundary.
fn gate_state_label(state: &GateState) -> String {
    match state {
        GateState::Idle => "idle".to_string(),
        GateState::InProgress { attempt } => format!("in_progress(attempt={attempt})"),
        GateState::Failed { attempt, .. } => format!("failed(attempt={attempt})"),
        GateState::PermanentlyFailed { .. } => "permanently_failed".to_string(),
    }
}

/// Current state of a [`RecoveryGate`].
#[non_exhaustive]
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
#[must_use = "dropping a RecoveryTicket without resolve/fail triggers auto-fail"]
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
        let next = Arc::new(GateState::Idle);
        // Store BEFORE emit so a subscriber that reacts to the event and
        // immediately reads `gate.state()` observes the new state, not
        // the old one. Matches the emit-after-CAS ordering used in
        // `try_begin` / `try_begin_from_failed` / `cas_to_permanently_failed`.
        self.gate.state.store(Arc::clone(&next));
        self.gate.emit_state(&next);
        self.gate.notify.notify_waiters();
    }

    /// Marks the recovery as transiently failed with exponential backoff.
    pub fn fail_transient(mut self, message: impl Into<String>) {
        self.consumed = true;
        let backoff = compute_backoff(self.gate.base_backoff, self.attempt);
        let next = Arc::new(GateState::Failed {
            message: message.into(),
            retry_at: Instant::now() + backoff,
            attempt: self.attempt,
        });
        // Store-then-emit ordering: see `resolve` for the rationale.
        self.gate.state.store(Arc::clone(&next));
        self.gate.emit_state(&next);
        self.gate.notify.notify_waiters();
    }

    /// Marks the recovery as permanently failed — no further attempts.
    pub fn fail_permanent(mut self, message: impl Into<String>) {
        self.consumed = true;
        let next = Arc::new(GateState::PermanentlyFailed {
            message: message.into(),
        });
        // Store-then-emit ordering: see `resolve` for the rationale.
        self.gate.state.store(Arc::clone(&next));
        self.gate.emit_state(&next);
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
            let next = Arc::new(GateState::Failed {
                message: "recovery ticket dropped without resolution".into(),
                retry_at: Instant::now() + backoff,
                attempt: self.attempt,
            });
            // Store-then-emit ordering: see `RecoveryTicket::resolve` for
            // the rationale (subscribers must observe the new state when
            // they react to the event).
            self.gate.state.store(Arc::clone(&next));
            self.gate.emit_state(&next);
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
    ///
    /// # Why this is lost-wakeup free without `enable()`
    ///
    /// The `Notified` future is constructed **before** the `state` snapshot
    /// is loaded, and every gate notifier wakes waiters with
    /// [`Notify::notify_waiters`] — never `notify_one` — paired with the
    /// `ArcSwap` state store at the same site (see [`RecoveryTicket::resolve`]
    /// / [`fail_transient`](RecoveryTicket::fail_transient) /
    /// [`fail_permanent`](RecoveryTicket::fail_permanent), the ticket `Drop`,
    /// [`RecoveryGate::reset`], and the max-attempts transition).
    ///
    /// Tokio's documented `notify_waiters` contract: `Notify::notified()`
    /// captures the current `notify_waiters` invocation count at the moment
    /// the future is *created*. A `notify_waiters()` call that lands after
    /// the future is created but before its first poll therefore advances
    /// that count and is delivered on the first poll — no prior poll and no
    /// `Notified::enable()` is required. So in the loop body, if a notifier
    /// fires in the window between `notified()` construction and the `.await`,
    /// either the freshly stored non-`InProgress` state is observed by the
    /// `load()` below (and we return immediately) or the captured-by-count
    /// notification satisfies the `.await` on first poll. No wakeup is lost
    /// and no spin is needed.
    ///
    /// This is distinct from the drain wait in `manager::shutdown`
    /// (`wait_for_tracker_drain`), which *does* need
    /// `Notified::enable()`: that loop re-reads an **external `AtomicU64`**
    /// the `Notify` does not gate, between waiter registration and the
    /// `.await`. `enable()` registers the waiter synchronously *before* that
    /// external re-check so a `notify_waiters()` firing in that gap is not
    /// missed. Here there is no separate post-registration value to re-read —
    /// the state transition and its `notify_waiters()` are captured by the
    /// creation-time count on the very `Notify` this future listens on — so
    /// the synchronous pre-registration `enable()` provides is unnecessary.
    /// Switching any notifier to `notify_one`, or loading the state before
    /// constructing the `Notified` future, would reintroduce a lost-wakeup
    /// window; the `#[cfg(test)]` correctness-pin below guards that ordering.
    pub async fn wait(&self) -> GateState {
        loop {
            // Register notification BEFORE checking state to avoid missing
            // a notify that fires between the state check and the await.
            let notified = self.gate.notify.notified();
            let snap = self.gate.state.load();
            if !matches!(**snap, GateState::InProgress { .. }) {
                return (**snap).clone();
            }
            notified.await;
        }
    }
}

/// Shared interior of [`RecoveryGate`].
struct RecoveryGateInner {
    state: ArcSwap<GateState>,
    notify: Notify,
    max_attempts: u32,
    base_backoff: Duration,
    /// Optional event sink attached once via [`RecoveryGate::set_event_sink`].
    /// `OnceLock` so the gate can be constructed before its owning manager
    /// (engine registrar, scoped registries) and wired up later without
    /// disturbing live waiters.
    event_sink: OnceLock<EventSink>,
}

impl RecoveryGateInner {
    /// Best-effort emit of [`ResourceEvent::RecoveryGateChanged`] for the
    /// new state. Silently drops the broadcast result when no subscribers
    /// are attached — same fail-open contract as every other
    /// `event_tx.send(...)` site in the manager.
    fn emit_state(&self, new_state: &GateState) {
        if let Some(sink) = self.event_sink.get() {
            sink.tx
                .send(ResourceEvent::RecoveryGateChanged {
                    key: sink.key.clone(),
                    state: gate_state_label(new_state),
                })
                .ok();
        }
    }
}

impl std::fmt::Debug for RecoveryGateInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryGateInner")
            .field("state", &*self.state.load())
            .field("max_attempts", &self.max_attempts)
            .field("base_backoff", &self.base_backoff)
            .field("event_sink_wired", &self.event_sink.get().is_some())
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
                event_sink: OnceLock::new(),
            }),
        }
    }

    /// Returns the base backoff this gate uses to compute attempt delays.
    /// Used by the manager's acquire pipeline to publish the expected
    /// post-failure backoff in [`ResourceEvent::RetryAttempt`].
    pub fn base_backoff(&self) -> Duration {
        self.inner.base_backoff
    }

    /// Returns the backoff that would be imposed if the attempt with the
    /// given 1-based index were to fail transiently. Mirrors the internal
    /// `compute_backoff` formula (`base * 2^(attempt - 1)`, capped at 5
    /// minutes) so callers can publish a truthful expected delay in
    /// [`ResourceEvent::RetryAttempt`] without reimplementing the math.
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        compute_backoff(self.inner.base_backoff, attempt)
    }

    /// Attaches an event sink so subsequent state transitions emit
    /// [`ResourceEvent::RecoveryGateChanged`]. Wired by
    /// [`Manager::register`](crate::Manager::register) at registration
    /// time; not part of the public crate surface — the
    /// `broadcast::Sender` is an internal transport detail the manager
    /// owns.
    ///
    /// **Idempotent** — the first call wins; subsequent calls silently
    /// no-op so a gate handed to multiple registries does not flip
    /// subscribers mid-flight.
    ///
    /// **One gate per resource registration.** A `RecoveryGate` shared
    /// across multiple `Manager::register` calls (e.g. a recovery group
    /// reused for both `db` and `cache`) will report every transition
    /// under the **first** registering resource's key — the second
    /// `set_event_sink` is a no-op by design. Treat one
    /// `Arc<RecoveryGate>` as belonging to one resource registration; if
    /// recovery-group sharing matters, give each resource its own gate.
    // TODO(eventbus-migration): convert to EventBus — currently unused
    // because `Manager::register` no longer wires a broadcast sender.
    #[allow(dead_code)]
    pub(crate) fn set_event_sink(&self, tx: broadcast::Sender<ResourceEvent>, key: ResourceKey) {
        // OnceLock::set returns Err on second call — we treat that as
        // a no-op rather than a programming error, since the manager
        // may re-register a resource that already had a gate wired.
        self.inner.event_sink.set(EventSink { tx, key }).ok();
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
                    let next_state = GateState::InProgress { attempt: 1 };
                    let next = Arc::new(next_state);
                    let prev = self
                        .inner
                        .state
                        .compare_and_swap(&current, Arc::clone(&next));
                    if Arc::ptr_eq(&prev, &current) {
                        // CAS succeeded — publish the transition. Emitting
                        // after the swap matches the manager's ordering
                        // discipline: observers receive the event only on a
                        // state that was actually durably stored.
                        self.inner.emit_state(&next);
                        return Ok(RecoveryTicket {
                            gate: Arc::clone(&self.inner),
                            attempt: 1,
                            consumed: false,
                        });
                    }
                    // CAS failed — retry the loop.
                },
                GateState::InProgress { .. } => {
                    return Err(TryBeginError::AlreadyInProgress(RecoveryWaiter {
                        gate: Arc::clone(&self.inner),
                    }));
                },
                GateState::Failed {
                    retry_at, attempt, ..
                } => {
                    if Instant::now() < *retry_at {
                        return Err(TryBeginError::RetryLater {
                            retry_at: *retry_at,
                        });
                    }
                    match self.try_begin_from_failed(&current, *attempt) {
                        Some(result) => return result,
                        None => continue, // CAS failed — retry the loop.
                    }
                },
                GateState::PermanentlyFailed { message } => {
                    return Err(TryBeginError::PermanentlyFailed {
                        message: message.clone(),
                    });
                },
            }
        }
    }

    /// Attempts a CAS transition from `Failed` to either `PermanentlyFailed`
    /// (when max attempts exceeded) or `InProgress`.
    ///
    /// Returns `Some(result)` on successful CAS, `None` if CAS failed and the
    /// caller should retry.
    fn try_begin_from_failed(
        &self,
        current: &arc_swap::Guard<Arc<GateState>>,
        attempt: u32,
    ) -> Option<Result<RecoveryTicket, TryBeginError>> {
        let next_attempt = attempt + 1;
        if next_attempt > self.inner.max_attempts {
            return self.cas_to_permanently_failed(current);
        }
        let next = Arc::new(GateState::InProgress {
            attempt: next_attempt,
        });
        let prev = self
            .inner
            .state
            .compare_and_swap(current, Arc::clone(&next));
        if !Arc::ptr_eq(&prev, current) {
            return None; // CAS failed
        }
        // CAS succeeded — publish the transition (see `try_begin` for the
        // emit-after-swap rationale).
        self.inner.emit_state(&next);
        Some(Ok(RecoveryTicket {
            gate: Arc::clone(&self.inner),
            attempt: next_attempt,
            consumed: false,
        }))
    }

    /// CAS transition to [`GateState::PermanentlyFailed`] when max attempts
    /// are exceeded.
    ///
    /// Returns `Some(Err(...))` on success, `None` if CAS failed.
    fn cas_to_permanently_failed(
        &self,
        current: &arc_swap::Guard<Arc<GateState>>,
    ) -> Option<Result<RecoveryTicket, TryBeginError>> {
        let msg = format!(
            "max recovery attempts ({}) exceeded",
            self.inner.max_attempts
        );
        let next = Arc::new(GateState::PermanentlyFailed {
            message: msg.clone(),
        });
        let prev = self
            .inner
            .state
            .compare_and_swap(current, Arc::clone(&next));
        if !Arc::ptr_eq(&prev, current) {
            return None; // CAS failed
        }
        // CAS succeeded — publish the transition (emit-after-swap, same
        // rationale as `try_begin`).
        self.inner.emit_state(&next);
        self.inner.notify.notify_waiters();
        Some(Err(TryBeginError::PermanentlyFailed { message: msg }))
    }

    /// Returns a snapshot of the current gate state.
    pub fn state(&self) -> GateState {
        (**self.inner.state.load()).clone()
    }

    /// Forces the gate back to [`GateState::Idle`] (admin override).
    pub fn reset(&self) {
        let next = Arc::new(GateState::Idle);
        // Store-then-emit ordering: see `RecoveryTicket::resolve` for the
        // rationale.
        self.inner.state.store(Arc::clone(&next));
        self.inner.emit_state(&next);
        self.inner.notify.notify_waiters();
    }
}

/// Computes exponential backoff: `base * 2^(attempt - 1)`, capped at 5 min.
fn compute_backoff(base: Duration, attempt: u32) -> Duration {
    let multiplier = 1u64
        .checked_shl(attempt.saturating_sub(1))
        .unwrap_or(u64::MAX);
    // Use u64 arithmetic to avoid truncation for large attempts.
    let backoff_millis = (base.as_millis() as u64).saturating_mul(multiplier);
    let max_millis = MAX_BACKOFF.as_millis() as u64;
    Duration::from_millis(backoff_millis.min(max_millis))
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
            Err(TryBeginError::AlreadyInProgress(_)) => {}, // expected
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
            },
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
            },
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
        let base = Duration::from_mins(1);
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
            },
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
                },
                Err(TryBeginError::AlreadyInProgress(_)) => {
                    waiters += 1;
                },
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

    /// Correctness-pin (NOT a RED-then-fix; no `enable()` change).
    ///
    /// `RecoveryWaiter::wait` creates the `Notified` future *before* loading
    /// the gate state, and every gate notifier uses `notify_waiters()` (not
    /// `notify_one()`). tokio's documented contract: `notified()` captures
    /// the `notify_waiters` call-count at creation, so a `notify_waiters()`
    /// firing between the future's creation and its first `.await` is
    /// delivered on first poll **without** `enable()` and without a prior
    /// poll. This test pins that exact property on a bare `Notify` mirroring
    /// `wait`'s construction order, so a future refactor that switches the
    /// gate to `notify_one()` (or reorders the create-then-load) would break
    /// this pin and surface the regression.
    #[tokio::test]
    async fn notify_waiters_between_notified_creation_and_await_is_delivered() {
        let notify = Notify::new();

        // Mirror `RecoveryWaiter::wait`: create the future first…
        let fut = notify.notified();

        // …then fire `notify_waiters()` *before* the future is ever polled
        // / awaited (no `enable()` call). Per the tokio contract this is
        // captured by the count taken at `notified()` creation.
        notify.notify_waiters();

        // The await must complete immediately on first poll — a missed
        // wakeup would hang here. A bounded timeout converts a contract
        // regression into a fast failure instead of a hung test.
        tokio::time::timeout(Duration::from_secs(1), fut)
            .await
            .expect(
                "notify_waiters() fired after notified() creation but before \
                 .await must be delivered without enable() (tokio contract \
                 — this is why RecoveryWaiter::wait's ordering is correct)",
            );
    }

    /// End-to-end companion: a `RecoveryWaiter` obtained while a ticket is
    /// held must unblock once the ticket resolves, even when `resolve()`
    /// races immediately after the waiter is constructed (the
    /// create-notified-before-state-load ordering is what guarantees no lost
    /// wakeup). Deterministic — no sleeps, no yield-budget guessing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovery_waiter_unblocks_when_resolve_races_the_wait() {
        let gate = default_gate();
        let ticket = gate.try_begin().expect("first caller wins the ticket");

        let waiter = match gate.try_begin() {
            Err(TryBeginError::AlreadyInProgress(w)) => w,
            other => panic!("expected AlreadyInProgress, got: {other:?}"),
        };

        // Resolve and wait are started "simultaneously": even if `resolve()`
        // (state→Idle + notify_waiters) lands between the waiter's
        // `notified()` creation and its `.await`, the wait must still
        // observe a non-`InProgress` state or receive the captured notify —
        // never hang.
        let wait_task = tokio::spawn(async move { waiter.wait().await });
        ticket.resolve();

        let state = tokio::time::timeout(Duration::from_secs(2), wait_task)
            .await
            .expect("waiter must not hang when resolve races the wait")
            .expect("wait task must not panic");
        assert!(
            matches!(state, GateState::Idle),
            "resolve returns the gate to Idle; the waiter must observe it"
        );
    }
}
