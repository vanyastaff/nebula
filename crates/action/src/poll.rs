//! [`PollAction`] trait and [`PollTriggerAdapter`] — periodic pull-based triggers.
//!
//! A poll trigger runs a cancel-safe loop driven by
//! [`PollAction::poll_interval`]: sleep → poll → emit events. The
//! adapter enforces a 100 ms interval floor, rate-limits warn-level
//! logs per failure kind, and uses an `AtomicBool` + RAII `StartedGuard`
//! to reject double-start without poisoning the sentinel.
//!
//! The cursor is in-memory only — across process restarts it resets to
//! `Default::default()`. Cross-restart persistence requires runtime
//! storage integration (post-v1).

use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    action::Action, capability::ActionLogLevel, context::TriggerContext, error::ActionError,
    metadata::ActionMetadata, trigger::TriggerHandler,
};

/// Minimum poll interval enforced by [`PollTriggerAdapter`].
///
/// Anything shorter is clamped to this floor and a warning is logged.
/// Rationale: 10 Hz is the upper bound of legitimate poll-based
/// integration with upstream APIs. Higher frequencies are either a
/// configuration bug (e.g., `Duration::ZERO` from a missing field) or
/// an abusive integration. Use a streaming trigger (post-v1) for real
/// high-frequency event sources.
pub const POLL_INTERVAL_FLOOR: Duration = Duration::from_millis(100);

// ── PollCycle + EmitFailurePolicy ─────────────────────────────────────────

/// Policy for handling event dispatch failures within a poll cycle.
///
/// Controls what happens when individual events from a [`PollCycle`]
/// fail to serialize or emit. The policy is set per cycle by the action,
/// allowing different strategies for different data sources.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmitFailurePolicy {
    /// Stop the trigger on the first dispatch failure. The cursor is
    /// NOT advanced — the next `start()` replays from the same position.
    /// This is the safe default.
    #[default]
    StopOnFailure,
    /// Drop failed events and continue. The cursor IS advanced to the
    /// returned position. Use for best-effort integrations where data
    /// loss is acceptable.
    DropAndContinue,
    /// Retry the entire cycle up to `max_retries` times, then stop.
    /// Each retry re-dispatches ALL events from this cycle (not just
    /// the failed ones).
    RetryThenStop {
        /// Maximum retry count before the trigger stops.
        max_retries: u32,
    },
}

/// Result of a single poll cycle, returned by [`PollAction::poll`].
///
/// Decouples the cursor update from event dispatch. The adapter
/// advances the cursor to [`next_cursor`](Self::next_cursor) only
/// after all events have been dispatched successfully (or per the
/// [`emit_failure_policy`](Self::emit_failure_policy)).
///
/// # Cursor checkpoint semantics
///
/// The action receives `&Self::Cursor` (immutable) and returns a new
/// cursor value inside `PollCycle`. The adapter stores this new cursor
/// only if dispatch succeeds. On failure with
/// [`EmitFailurePolicy::StopOnFailure`], the cursor stays at the
/// previous position, enabling safe replay on restart.
#[derive(Debug)]
pub struct PollCycle<C, E> {
    /// Updated cursor position. Applied by the adapter after dispatch.
    pub next_cursor: C,
    /// Events to emit as workflow executions.
    pub events: Vec<E>,
    /// How the adapter handles dispatch failures for this cycle.
    pub emit_failure_policy: EmitFailurePolicy,
}

impl<C, E> PollCycle<C, E> {
    /// Create a cycle with events and default policy (`StopOnFailure`).
    pub fn new(next_cursor: C, events: Vec<E>) -> Self {
        Self {
            next_cursor,
            events,
            emit_failure_policy: EmitFailurePolicy::default(),
        }
    }

    /// Override the emit failure policy.
    #[must_use]
    pub fn with_policy(mut self, policy: EmitFailurePolicy) -> Self {
        self.emit_failure_policy = policy;
        self
    }
}

/// Minimum interval between repeated warn-level log lines for the same
/// failure kind inside a [`PollTriggerAdapter`].
///
/// A stuck trigger that fails every cycle would otherwise flood logs
/// with one line per poll — at `POLL_INTERVAL_FLOOR` = 100 ms that is
/// 600 lines/minute per trigger. 30 s cooldown keeps the first hit
/// visible, suppresses the following flood, and re-emits after the
/// window so recurring failures are still observable.
const POLL_WARN_COOLDOWN: Duration = Duration::from_secs(30);

/// Periodic polling trigger with in-memory cursor.
///
/// Implement `poll_interval` and `poll`, then register via
/// `registry.register_poll(action)`.
///
/// The [`PollTriggerAdapter`] runs a blocking loop in `start()`:
/// sleep → poll → emit events. Cancellation via `TriggerContext::cancellation`.
///
/// **Note:** The cursor is in-memory only. Across process restarts the cursor
/// resets to `Default::default()` — full persistence requires runtime storage
/// integration (post-v1).
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::poll::{PollAction, PollCycle};
///
/// struct RssPoll { feed_url: String }
/// impl PollAction for RssPoll {
///     type Cursor = String;
///     type Event = serde_json::Value;
///     fn poll_interval(&self) -> std::time::Duration { std::time::Duration::from_secs(300) }
///     async fn poll(&self, cursor: &String, ctx: &TriggerContext)
///         -> Result<PollCycle<String, serde_json::Value>, ActionError> {
///         let (items, new_cursor) = fetch_rss(&self.feed_url, cursor).await?;
///         Ok(PollCycle::new(new_cursor, items))
///     }
/// }
/// ```
pub trait PollAction: Action + Send + Sync + 'static {
    /// Cursor type for tracking poll position.
    type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    /// Event type emitted per poll cycle (each becomes a workflow execution).
    type Event: Serialize + Send + Sync;

    /// Interval between poll cycles.
    fn poll_interval(&self) -> Duration;

    /// Maximum duration for a single `poll()` call before the adapter
    /// treats it as timed out (retryable error). Default: 30 seconds.
    ///
    /// Override for slow upstream APIs or when the action performs
    /// expensive pagination within a single poll cycle.
    fn poll_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    /// Execute one poll cycle.
    ///
    /// Receives the current cursor by shared reference. Return a
    /// [`PollCycle`] containing the updated cursor and events to emit.
    /// The adapter advances the cursor only after successful dispatch.
    ///
    /// # Errors
    ///
    /// [`ActionError::Retryable`] for transient failures (skip this cycle),
    /// [`ActionError::Fatal`] to stop the trigger permanently.
    fn poll(
        &self,
        cursor: &Self::Cursor,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<PollCycle<Self::Cursor, Self::Event>, ActionError>> + Send;
}

/// Rate-limits adapter warn lines to avoid log flooding.
///
/// Holds the timestamp of the last emitted warning. `should_log()`
/// returns `true` the first time it is called, then `false` until
/// `POLL_WARN_COOLDOWN` has elapsed, then `true` again. Each failure
/// kind (poll error, serialize error, emit error) gets its own
/// throttle so they cannot starve each other.
struct WarnThrottle {
    last_logged: parking_lot::Mutex<Option<Instant>>,
}

impl WarnThrottle {
    fn new() -> Self {
        Self {
            last_logged: parking_lot::Mutex::new(None),
        }
    }

    /// Returns `true` if the caller should emit a log line; updates
    /// the last-logged timestamp on a yes. Safe to call concurrently.
    fn should_log(&self) -> bool {
        let mut guard = self.last_logged.lock();
        let now = Instant::now();
        match *guard {
            Some(last) if now.duration_since(last) < POLL_WARN_COOLDOWN => false,
            _ => {
                *guard = Some(now);
                true
            }
        }
    }
}

/// RAII guard that clears the `started` flag when `start()` exits.
///
/// Uses the "defused" pattern — a plain `Drop` impl with no `mem::forget`
/// call — so the flag is correctly cleared on panic, cancellation, or
/// normal return. Keeping the flag set across a crashed `start()` would
/// permanently poison the adapter.
struct StartedGuard<'a>(&'a AtomicBool);

impl Drop for StartedGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Wraps a [`PollAction`] as a [`dyn TriggerHandler`].
///
/// `start()` runs a blocking loop: sleep → poll → emit events.
/// Cancellation via `TriggerContext::cancellation`.
/// `stop()` is a no-op (cancellation token handles shutdown).
///
/// Created automatically by `nebula_runtime::ActionRegistry::register_poll`.
///
/// # Double-start rejection
///
/// `start()` uses an `AtomicBool` sentinel to reject a concurrent or
/// sequential second call with `ActionError::Fatal`. The sentinel is
/// cleared via an RAII guard (defused pattern, NOT `mem::forget`) when
/// `start()` exits — whether by cancellation, error, or normal return —
/// so `stop() → start()` restart works.
///
/// # Error handling
///
/// - `ActionError::Fatal` from `poll()` stops the loop immediately
/// - `ActionError::Retryable` skips the current cycle, continues at next interval
/// - Emit failures are silently dropped (transient emitter issues don't kill the trigger)
pub struct PollTriggerAdapter<A: PollAction> {
    action: A,
    /// `true` while `start()` is running. Cleared by `StartedGuard` on exit.
    started: AtomicBool,
    /// Rate-limits retryable-poll-error warnings.
    poll_warn: WarnThrottle,
    /// Rate-limits event-serialization-failure warnings.
    serialize_warn: WarnThrottle,
    /// Rate-limits emitter.emit failure warnings.
    emit_warn: WarnThrottle,
}

impl<A: PollAction> PollTriggerAdapter<A> {
    /// Wrap a typed poll action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self {
            action,
            started: AtomicBool::new(false),
            poll_warn: WarnThrottle::new(),
            serialize_warn: WarnThrottle::new(),
            emit_warn: WarnThrottle::new(),
        }
    }

    /// Serialize and emit a batch of events. Returns `true` if all
    /// events dispatched successfully, `false` if any failed.
    ///
    /// The `policy` parameter controls failure handling:
    /// - `DropAndContinue` — log and skip failed events, return `true`
    /// - `StopOnFailure` / `RetryThenStop` — return `false` on first failure
    async fn dispatch_events(
        &self,
        events: &[A::Event],
        policy: EmitFailurePolicy,
        ctx: &TriggerContext,
    ) -> bool
    where
        A::Event: Send + Sync,
    {
        let action_key = &self.action.metadata().key;
        for event in events {
            let payload = match serde_json::to_value(event) {
                Ok(v) => v,
                Err(e) => {
                    if self.serialize_warn.should_log() {
                        ctx.logger.log(
                            ActionLogLevel::Warn,
                            &format!("poll trigger {action_key}: event serialization failed: {e}"),
                        );
                    }
                    match policy {
                        EmitFailurePolicy::DropAndContinue => continue,
                        _ => return false,
                    }
                }
            };
            if let Err(e) = ctx.emitter.emit(payload).await {
                if self.emit_warn.should_log() {
                    ctx.logger.log(
                        ActionLogLevel::Warn,
                        &format!("poll trigger {action_key}: emitter.emit failed: {e}"),
                    );
                }
                match policy {
                    EmitFailurePolicy::DropAndContinue => continue,
                    _ => return false,
                }
            }
        }
        true
    }
}

#[async_trait]
impl<A> TriggerHandler for PollTriggerAdapter<A>
where
    A: PollAction + Send + Sync + 'static,
    A::Cursor: Send + Sync,
    A::Event: Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    /// Run the poll loop until cancellation.
    ///
    /// # Cancel safety
    ///
    /// The internal `tokio::select!` races two futures:
    ///
    /// 1. `ctx.cancellation.cancelled()` — a `CancellationToken` future that is cancel-safe:
    ///    dropping it mid-poll simply unregisters the waker.
    /// 2. `tokio::time::sleep(interval)` — the `Sleep` future from tokio is cancel-safe: dropping
    ///    it mid-wait cancels the timer with no observable effect on adjacent state.
    ///
    /// Neither branch holds a lock, a guard, or state that would
    /// leak on drop. The only state mutated inside the poll branch
    /// is `cursor`, which `PollAction::poll` owns and the loop
    /// re-reads on the next iteration — if cancellation fires
    /// between `sleep` completing and `poll` being awaited, the
    /// cursor simply stays at its previous position and the loop
    /// exits cleanly.
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        // Atomically claim the "started" slot. If another call already
        // owns it, fail rather than spawning a second poll loop against
        // a shared cursor — that would double-emit every event.
        if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ActionError::fatal(
                "poll trigger already started; call stop() before start() again",
            ));
        }

        // RAII: clear the flag on every exit path (cancellation, error,
        // normal return) so restart works. Dropped at end of scope.
        let _guard = StartedGuard(&self.started);

        let mut cursor = A::Cursor::default();
        let raw_interval = self.action.poll_interval();
        // Floor the interval at 100 ms. Anything shorter is a busy-loop bug
        // or an abusive integration — Nebula is a workflow engine, not an
        // HFT platform. Legitimate high-frequency polling should use a
        // streaming trigger, not a poll trigger.
        let interval = raw_interval.max(POLL_INTERVAL_FLOOR);
        if interval != raw_interval {
            // One-shot event per start() invocation — no throttle needed,
            // `StartedGuard` ensures we only reach this branch once per
            // start cycle. Routed through `ctx.logger` for the same reason
            // as the three per-cycle error warnings below: tests assert
            // over `SpyLogger`, production routes through the configured
            // logger sink rather than the global tracing subscriber.
            ctx.logger.log(
                ActionLogLevel::Warn,
                &format!(
                    "poll trigger {}: poll_interval below floor; \
                     requested {:?}, clamped to {:?} to prevent busy loop",
                    self.action.metadata().key,
                    raw_interval,
                    interval,
                ),
            );
        }

        loop {
            tokio::select! {
                () = ctx.cancellation.cancelled() => {
                    return Ok(());
                }
                () = tokio::time::sleep(interval) => {
                    let poll_result = match tokio::time::timeout(
                        self.action.poll_timeout(),
                        self.action.poll(&cursor, ctx),
                    ).await {
                        Ok(result) => result,
                        Err(_elapsed) => {
                            Err(ActionError::retryable(format!(
                                "poll trigger {}: poll() timed out after {:?}",
                                self.action.metadata().key,
                                self.action.poll_timeout(),
                            )))
                        }
                    };
                    match poll_result {
                        Ok(cycle) => {
                            let policy = cycle.emit_failure_policy;
                            if cycle.events.is_empty() {
                                cursor = cycle.next_cursor;
                            } else {
                                match policy {
                                    EmitFailurePolicy::StopOnFailure => {
                                        if self.dispatch_events(&cycle.events, policy, ctx).await {
                                            cursor = cycle.next_cursor;
                                        } else {
                                            return Err(ActionError::fatal(format!(
                                                "poll trigger {}: dispatch failed with StopOnFailure policy",
                                                self.action.metadata().key,
                                            )));
                                        }
                                    }
                                    EmitFailurePolicy::DropAndContinue => {
                                        self.dispatch_events(&cycle.events, policy, ctx).await;
                                        cursor = cycle.next_cursor;
                                    }
                                    EmitFailurePolicy::RetryThenStop { max_retries } => {
                                        let mut success = false;
                                        for attempt in 0..=max_retries {
                                            if self.dispatch_events(&cycle.events, policy, ctx).await {
                                                success = true;
                                                break;
                                            }
                                            if attempt < max_retries && self.poll_warn.should_log() {
                                                ctx.logger.log(
                                                    ActionLogLevel::Warn,
                                                    &format!(
                                                        "poll trigger {}: dispatch retry {}/{max_retries}",
                                                        self.action.metadata().key,
                                                        attempt + 1,
                                                    ),
                                                );
                                            }
                                        }
                                        if success {
                                            cursor = cycle.next_cursor;
                                        } else {
                                            return Err(ActionError::fatal(format!(
                                                "poll trigger {}: dispatch failed after {max_retries} retries",
                                                self.action.metadata().key,
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) if e.is_fatal() => return Err(e),
                        Err(e) => {
                            if self.poll_warn.should_log() {
                                let action_key = &self.action.metadata().key;
                                ctx.logger.log(
                                    ActionLogLevel::Warn,
                                    &format!(
                                        "poll trigger {action_key}: retryable poll error, \
                                         will retry next cycle: {e}"
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// No-op — poll shutdown is driven by `ctx.cancellation`.
    ///
    /// [`PollTriggerAdapter::start`] runs an internal loop that exits
    /// when the cancellation token fires. Calling `stop()` explicitly
    /// is not required; the [`StartedGuard`] RAII pattern clears the
    /// `started` flag on every exit path (including cancellation).
    ///
    /// Callers that need to ensure the poll loop has fully exited
    /// should cancel the token and then `.await` the `start()` task.
    async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
        Ok(())
    }
}

impl<A: PollAction> std::fmt::Debug for PollTriggerAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollTriggerAdapter")
            .field("action", &self.action.metadata().key)
            .finish_non_exhaustive()
    }
}
