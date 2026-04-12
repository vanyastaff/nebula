//! [`PollAction`] trait and [`PollTriggerAdapter`] — periodic pull-based triggers.
//!
//! A poll trigger runs a cancel-safe loop driven by [`PollConfig`]:
//! sleep → poll → emit events. The adapter owns all timing logic —
//! backoff, jitter, timeout, interval floor — so the action author
//! never writes a sleep loop.
//!
//! The cursor is in-memory only — across process restarts it resets to
//! `Default::default()`. Cross-restart persistence requires runtime
//! storage integration (post-v1).

use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    action::Action, capability::ActionLogLevel, context::TriggerContext, error::ActionError,
    metadata::ActionMetadata, trigger::TriggerHandler,
};

/// Minimum poll interval enforced by [`PollTriggerAdapter`].
///
/// The computed sleep duration (after backoff + jitter) is clamped to
/// this floor. Rationale: 10 Hz is the upper bound of legitimate
/// poll-based integration. Higher frequencies are a configuration bug
/// or an abusive integration.
pub const POLL_INTERVAL_FLOOR: Duration = Duration::from_millis(100);

// ── PollConfig ────────────────────────────────────────────────────────────

/// Declarative polling configuration.
///
/// Returned by [`PollAction::poll_config`] once at `start()`. The
/// adapter owns all timing logic — backoff on idle, jitter to prevent
/// thundering herd, timeout, and dispatch failure policy.
///
/// # Defaults
///
/// `PollConfig::default()` gives a fixed 60 s interval, no backoff, no
/// jitter, 30 s poll timeout, and `DropAndContinue` on emit failure.
/// Use [`PollConfig::fixed`] for a drop-in replacement of the old
/// `poll_interval()` pattern.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PollConfig {
    /// Interval used immediately after a non-empty poll result (or on
    /// the first cycle). Backoff multiplies this value on consecutive
    /// empty polls.
    pub base_interval: Duration,

    /// Upper bound — backoff never exceeds this.
    pub max_interval: Duration,

    /// Multiplier applied per consecutive empty poll. `1.0` = fixed
    /// interval (no backoff).
    pub backoff_factor: f64,

    /// Random ±fraction (0.0–0.5) applied to the computed interval to
    /// prevent thundering herd when many triggers share a schedule.
    pub jitter: f64,

    /// Maximum duration for a single `poll()` call. If exceeded, the
    /// adapter treats it as a retryable error and moves to the next
    /// cycle.
    pub poll_timeout: Duration,

    /// What happens when `emitter.emit()` or serialization fails for
    /// events within a batch.
    pub emit_failure: EmitFailurePolicy,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            base_interval: Duration::from_secs(60),
            max_interval: Duration::from_secs(3600),
            backoff_factor: 1.0,
            jitter: 0.0,
            poll_timeout: Duration::from_secs(30),
            emit_failure: EmitFailurePolicy::default(),
        }
    }
}

impl PollConfig {
    /// Fixed interval, no backoff. Drop-in replacement for the old
    /// `poll_interval()` pattern.
    #[must_use]
    pub fn fixed(interval: Duration) -> Self {
        Self {
            base_interval: interval,
            max_interval: interval,
            ..Default::default()
        }
    }

    /// Exponential backoff on idle, reset on activity. Includes 10%
    /// jitter by default.
    #[must_use]
    pub fn with_backoff(base: Duration, max: Duration, factor: f64) -> Self {
        Self {
            base_interval: base,
            max_interval: max,
            backoff_factor: factor.max(1.0),
            jitter: 0.1,
            ..Default::default()
        }
    }

    /// Set jitter fraction (clamped to 0.0–0.5).
    #[must_use]
    pub fn with_jitter(mut self, jitter: f64) -> Self {
        self.jitter = jitter.clamp(0.0, 0.5);
        self
    }

    /// Set poll timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.poll_timeout = timeout;
        self
    }

    /// Set emit failure policy.
    #[must_use]
    pub fn with_emit_failure(mut self, policy: EmitFailurePolicy) -> Self {
        self.emit_failure = policy;
        self
    }
}

// ── EmitFailurePolicy ─────────────────────────────────────────────────────

/// What the adapter does when event dispatch (serialization or emit)
/// fails for some events in a batch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmitFailurePolicy {
    /// Drop failed events, advance cursor. Best-effort — suitable for
    /// most integrations where occasional event loss is acceptable.
    #[default]
    DropAndContinue,
    /// Restore cursor to pre-poll position so the next cycle re-fetches
    /// the same data. Use for payment/audit integrations where data
    /// loss is unacceptable.
    RetryBatch,
    /// Stop the trigger entirely on the first dispatch failure. Use
    /// when partial processing is worse than no processing.
    StopTrigger,
}

// ── PollResult ────────────────────────────────────────────────────────────

/// What [`PollAction::poll`] returns.
///
/// Contains the events to emit and an optional per-cycle interval
/// override (e.g., for `Retry-After` headers from rate-limited APIs).
#[derive(Debug)]
pub struct PollResult<E> {
    /// Events to emit as workflow executions.
    pub events: Vec<E>,
    /// One-shot override for the next sleep interval. Useful when the
    /// upstream API provides a `Retry-After` or backoff hint. `None`
    /// means use the normal computed interval.
    pub override_next: Option<Duration>,
}

impl<E> PollResult<E> {
    /// Create a result with events and no interval override.
    pub fn new(events: Vec<E>) -> Self {
        Self {
            events,
            override_next: None,
        }
    }

    /// Set a one-shot interval override for the next cycle.
    #[must_use]
    pub fn with_override_next(mut self, interval: Duration) -> Self {
        self.override_next = Some(interval);
        self
    }
}

impl<E> From<Vec<E>> for PollResult<E> {
    fn from(events: Vec<E>) -> Self {
        Self::new(events)
    }
}

// ── PollAction trait ──────────────────────────────────────────────────────

/// Periodic polling trigger with in-memory cursor.
///
/// Implement `poll_config` and `poll`, then register via
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
/// use nebula_action::poll::{PollAction, PollConfig, PollResult};
///
/// struct RssPoll { feed_url: String }
/// impl PollAction for RssPoll {
///     type Cursor = String;
///     type Event = serde_json::Value;
///
///     fn poll_config(&self) -> PollConfig {
///         PollConfig::with_backoff(
///             Duration::from_secs(60),
///             Duration::from_secs(3600),
///             2.0,
///         ).with_timeout(Duration::from_secs(10))
///     }
///
///     async fn poll(&self, cursor: &mut String, ctx: &TriggerContext)
///         -> Result<PollResult<serde_json::Value>, ActionError> {
///         let items = fetch_rss(&self.feed_url, cursor).await?;
///         Ok(items.into())
///     }
/// }
/// ```
pub trait PollAction: Action + Send + Sync + 'static {
    /// Cursor type for tracking poll position.
    type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    /// Event type emitted per poll cycle (each becomes a workflow execution).
    type Event: Serialize + Send + Sync;

    /// Polling configuration — interval, backoff, timeout, failure policy.
    ///
    /// Called once at `start()`. The adapter caches the result and owns
    /// all timing decisions.
    fn poll_config(&self) -> PollConfig;

    /// Execute one poll cycle. Mutate cursor to track position.
    ///
    /// Return events to emit. Empty vec = nothing new (triggers backoff
    /// if configured).
    ///
    /// # Errors
    ///
    /// [`ActionError::Retryable`] for transient failures (skip this cycle),
    /// [`ActionError::Fatal`] to stop the trigger permanently.
    fn poll(
        &self,
        cursor: &mut Self::Cursor,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<PollResult<Self::Event>, ActionError>> + Send;
}

// ── Internals ─────────────────────────────────────────────────────────────

/// Minimum interval between repeated warn-level log lines for the same
/// failure kind inside a [`PollTriggerAdapter`].
const POLL_WARN_COOLDOWN: Duration = Duration::from_secs(30);

struct WarnThrottle {
    last_logged: parking_lot::Mutex<Option<Instant>>,
}

impl WarnThrottle {
    fn new() -> Self {
        Self {
            last_logged: parking_lot::Mutex::new(None),
        }
    }

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

struct StartedGuard<'a>(&'a AtomicBool);

impl Drop for StartedGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Compute the sleep duration for the next cycle.
fn compute_interval(config: &PollConfig, consecutive_empty: u32) -> Duration {
    let base = config.base_interval.as_secs_f64();
    let multiplier = config.backoff_factor.powi(consecutive_empty as i32);
    let raw = base * multiplier;
    let capped = raw.min(config.max_interval.as_secs_f64());
    let with_jitter = apply_jitter(capped, config.jitter);
    Duration::from_secs_f64(with_jitter).max(POLL_INTERVAL_FLOOR)
}

fn apply_jitter(secs: f64, jitter_fraction: f64) -> f64 {
    if jitter_fraction <= 0.0 {
        return secs;
    }
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let normalized = f64::from(seed) / f64::from(u32::MAX);
    let jitter = (normalized * 2.0 - 1.0) * jitter_fraction;
    (secs * (1.0 + jitter)).max(0.001)
}

// ── PollTriggerAdapter ────────────────────────────────────────────────────

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
/// # Backoff
///
/// Consecutive empty poll results (no events) increase the sleep
/// interval by `backoff_factor` up to `max_interval`. A non-empty
/// result resets to `base_interval`.
///
/// # Cursor checkpoint
///
/// With [`EmitFailurePolicy::RetryBatch`], the adapter clones the
/// cursor before calling `poll()` and restores it if dispatch fails.
/// The action re-fetches the same data on the next cycle.
pub struct PollTriggerAdapter<A: PollAction> {
    action: A,
    started: AtomicBool,
    poll_warn: WarnThrottle,
    serialize_warn: WarnThrottle,
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

    /// Returns `true` if all events dispatched, `false` if any failed.
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
                    if policy == EmitFailurePolicy::DropAndContinue {
                        continue;
                    }
                    return false;
                }
            };
            if let Err(e) = ctx.emitter.emit(payload).await {
                if self.emit_warn.should_log() {
                    ctx.logger.log(
                        ActionLogLevel::Warn,
                        &format!("poll trigger {action_key}: emitter.emit failed: {e}"),
                    );
                }
                if policy == EmitFailurePolicy::DropAndContinue {
                    continue;
                }
                return false;
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

    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ActionError::fatal(
                "poll trigger already started; call stop() before start() again",
            ));
        }
        let _guard = StartedGuard(&self.started);

        let config = self.action.poll_config();
        let mut cursor = A::Cursor::default();
        let mut consecutive_empty: u32 = 0;
        let mut override_next: Option<Duration> = None;

        if config.base_interval < POLL_INTERVAL_FLOOR {
            ctx.logger.log(
                ActionLogLevel::Warn,
                &format!(
                    "poll trigger {}: base_interval below floor; \
                     requested {:?}, will be clamped to {:?}",
                    self.action.metadata().key,
                    config.base_interval,
                    POLL_INTERVAL_FLOOR,
                ),
            );
        }

        loop {
            let interval = override_next
                .take()
                .map(|d| d.max(POLL_INTERVAL_FLOOR))
                .unwrap_or_else(|| compute_interval(&config, consecutive_empty));

            tokio::select! {
                () = ctx.cancellation.cancelled() => {
                    return Ok(());
                }
                () = tokio::time::sleep(interval) => {
                    // Clone cursor for RetryBatch rollback.
                    let saved_cursor = if config.emit_failure == EmitFailurePolicy::RetryBatch {
                        Some(cursor.clone())
                    } else {
                        None
                    };

                    let poll_result = match tokio::time::timeout(
                        config.poll_timeout,
                        self.action.poll(&mut cursor, ctx),
                    ).await {
                        Ok(result) => result,
                        Err(_elapsed) => {
                            Err(ActionError::retryable(format!(
                                "poll trigger {}: poll() timed out after {:?}",
                                self.action.metadata().key,
                                config.poll_timeout,
                            )))
                        }
                    };

                    match poll_result {
                        Ok(result) => {
                            override_next = result.override_next;

                            if result.events.is_empty() {
                                consecutive_empty = consecutive_empty.saturating_add(1);
                            } else {
                                consecutive_empty = 0;
                                let ok = self.dispatch_events(
                                    &result.events,
                                    config.emit_failure,
                                    ctx,
                                ).await;

                                if !ok {
                                    match config.emit_failure {
                                        EmitFailurePolicy::DropAndContinue => {
                                            // unreachable: dispatch returns true
                                        }
                                        EmitFailurePolicy::RetryBatch => {
                                            if let Some(saved) = saved_cursor {
                                                cursor = saved;
                                            }
                                        }
                                        EmitFailurePolicy::StopTrigger => {
                                            return Err(ActionError::fatal(format!(
                                                "poll trigger {}: dispatch failed, \
                                                 stopping (StopTrigger policy)",
                                                self.action.metadata().key,
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) if e.is_fatal() => return Err(e),
                        Err(e) => {
                            // Retryable: restore cursor if RetryBatch.
                            if let Some(saved) = saved_cursor {
                                cursor = saved;
                            }
                            consecutive_empty = consecutive_empty.saturating_add(1);
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
