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
    collections::{HashSet, VecDeque},
    future::Future,
    hash::Hash,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
    ///
    /// **At-least-once semantics:** if dispatch partially succeeds
    /// (some events emitted before the failure), those events are NOT
    /// rolled back. The cursor rollback causes the next poll to
    /// re-fetch the same batch, producing duplicates for the events
    /// that were already emitted. Downstream consumers must be
    /// idempotent.
    RetryBatch,
    /// Stop the trigger entirely on the first dispatch failure. Use
    /// when partial processing is worse than no processing. Note:
    /// events dispatched before the failure are NOT rolled back —
    /// partial batch delivery is possible.
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

// ── DeduplicatingCursor ───────────────────────────────────────────────────

const DEFAULT_MAX_SEEN: usize = 1_000;

/// Cursor wrapper that tracks seen event keys to prevent duplicates
/// caused by timestamp granularity.
///
/// Many upstream APIs return timestamps rounded to the second or
/// minute. Events at the boundary between two poll cycles can appear
/// in both. `DeduplicatingCursor` maintains a bounded set of seen
/// keys alongside the inner cursor so the action can filter them out.
///
/// Uses `HashSet` for O(1) membership checks and `VecDeque` for FIFO
/// eviction order. The `key_fn` passed to [`filter_new`](Self::filter_new)
/// is called once per item regardless of whether it's new or seen.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::poll::DeduplicatingCursor;
///
/// type Cursor = DeduplicatingCursor<String, chrono::DateTime<chrono::Utc>>;
///
/// async fn poll(&self, cursor: &mut Cursor, ctx: &TriggerContext)
///     -> Result<PollResult<Event>, ActionError>
/// {
///     let raw_events = fetch_since(cursor.inner).await?;
///     let new_events = cursor.filter_new(raw_events, |e| e.id.clone());
///     if let Some(last) = new_events.last() {
///         cursor.inner = last.timestamp;
///     }
///     Ok(new_events.into())
/// }
/// ```
///
/// # State growth
///
/// The seen set is capped at `max_seen` (default 1,000). When the
/// cap is exceeded, the oldest entries are evicted (FIFO). Downstream
/// consumers should be idempotent as a defense-in-depth measure.
///
/// The entire cursor (including the seen set) is cloned once per poll
/// cycle for rollback safety. Keep `K` lightweight — prefer `String`
/// or integer IDs over large composite keys. For integrations that
/// need > 10k dedup keys, consider a custom cursor with
/// application-specific eviction.
#[derive(Debug, Clone)]
pub struct DeduplicatingCursor<K, C> {
    /// The inner cursor (e.g., timestamp, offset, revision ID).
    pub inner: C,
    /// Insertion-ordered keys for FIFO eviction. O(1) pop_front.
    order: VecDeque<K>,
    /// O(1) membership lookup. Rebuilt from `order` on deserialization.
    lookup: HashSet<K>,
    max_seen: usize,
}

fn default_max_seen() -> usize {
    DEFAULT_MAX_SEEN
}

// Custom Serialize: write `order` as `seen` (skip the HashSet).
impl<K: Serialize, C: Serialize> Serialize for DeduplicatingCursor<K, C> {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = ser.serialize_struct("DeduplicatingCursor", 3)?;
        s.serialize_field("inner", &self.inner)?;
        s.serialize_field("seen", &self.order)?;
        s.serialize_field("max_seen", &self.max_seen)?;
        s.end()
    }
}

// Custom Deserialize: read `seen` into VecDeque, rebuild HashSet.
impl<'de, K, C> Deserialize<'de> for DeduplicatingCursor<K, C>
where
    K: Deserialize<'de> + Hash + Eq + Clone,
    C: Deserialize<'de>,
{
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire<K, C> {
            inner: C,
            seen: VecDeque<K>,
            #[serde(default = "default_max_seen")]
            max_seen: usize,
        }
        let wire = Wire::<K, C>::deserialize(de)?;
        let lookup = wire.seen.iter().cloned().collect();
        Ok(Self {
            inner: wire.inner,
            order: wire.seen,
            lookup,
            max_seen: wire.max_seen,
        })
    }
}

impl<K: Hash + Eq + Clone, C: Default> Default for DeduplicatingCursor<K, C> {
    fn default() -> Self {
        Self {
            inner: C::default(),
            order: VecDeque::new(),
            lookup: HashSet::new(),
            max_seen: DEFAULT_MAX_SEEN,
        }
    }
}

impl<K: Hash + Eq + Clone, C> DeduplicatingCursor<K, C> {
    /// Create with a custom inner cursor and default cap (1,000).
    pub fn new(inner: C) -> Self {
        Self {
            inner,
            order: VecDeque::new(),
            lookup: HashSet::new(),
            max_seen: DEFAULT_MAX_SEEN,
        }
    }

    /// Set the maximum number of seen keys to retain.
    #[must_use]
    pub fn with_max_seen(mut self, max: usize) -> Self {
        self.max_seen = max.max(1);
        self
    }

    /// Check whether a key has NOT been seen before.
    pub fn is_new(&self, key: &K) -> bool {
        !self.lookup.contains(key)
    }

    /// Mark a key as seen. No-op if already present.
    pub fn mark_seen(&mut self, key: K) {
        self.try_insert(key);
    }

    /// Filter a batch of items, keeping only those with unseen keys.
    /// Seen keys are recorded automatically. This is the primary API
    /// for poll actions — call it once per cycle on the raw fetch
    /// results.
    ///
    /// `key_fn` is called once per item (including already-seen items).
    pub fn filter_new<T>(&mut self, items: Vec<T>, key_fn: impl Fn(&T) -> K) -> Vec<T> {
        let mut result = Vec::new();
        for item in items {
            let key = key_fn(&item);
            if self.try_insert(key) {
                result.push(item);
            }
        }
        result
    }

    /// Number of currently tracked keys.
    pub fn seen_count(&self) -> usize {
        self.lookup.len()
    }

    /// Clear all tracked keys without resetting the inner cursor.
    pub fn clear_seen(&mut self) {
        self.order.clear();
        self.lookup.clear();
    }

    /// Insert a key if not present. Returns `true` if the key was new.
    /// Evicts the oldest entry if the cap is exceeded.
    fn try_insert(&mut self, key: K) -> bool {
        if self.lookup.contains(&key) {
            return false;
        }
        self.order.push_back(key.clone());
        self.lookup.insert(key);
        // Evict oldest (FIFO) if over cap — O(1) per eviction.
        while self.lookup.len() > self.max_seen {
            if let Some(old) = self.order.pop_front() {
                self.lookup.remove(&old);
            }
        }
        true
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
    ///
    /// Cloned once per poll cycle for rollback safety — keep lightweight.
    type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    /// Event type emitted per poll cycle (each becomes a workflow execution).
    type Event: Serialize + Send + Sync;

    /// Polling configuration — interval, backoff, timeout, failure policy.
    ///
    /// Called once at `start()`. The adapter caches the result and owns
    /// all timing decisions.
    fn poll_config(&self) -> PollConfig;

    /// Validate configuration and connectivity before the poll loop.
    ///
    /// Called once by the adapter at the beginning of `start()`, before
    /// the first sleep/poll cycle. Return `Err` to abort activation
    /// (e.g., bad credentials, unreachable endpoint). The error
    /// propagates from `start()` as a fatal failure.
    ///
    /// Default: no-op (always succeeds).
    fn validate(
        &self,
        _ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    /// Execute one poll cycle. Mutate cursor to track position.
    ///
    /// Return events to emit. Empty vec = nothing new (triggers backoff
    /// if configured).
    ///
    /// # Cursor safety
    ///
    /// The adapter clones the cursor before calling `poll()` and
    /// restores the snapshot on any error. This means partial cursor
    /// mutations inside a failed `poll()` are rolled back automatically.
    /// Actions do not need to manually undo cursor changes on error.
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
///
/// Backoff covers both "no data" and "transient error" cases:
/// `consecutive_empty` increments on empty results AND retryable
/// errors, so the adapter backs off when the upstream is down too.
fn compute_interval(config: &PollConfig, consecutive_empty: u32, identity_seed: u64) -> Duration {
    let base = config.base_interval.as_secs_f64();
    let multiplier = config.backoff_factor.powi(consecutive_empty as i32);
    let raw = base * multiplier;
    let capped = raw.min(config.max_interval.as_secs_f64());
    let with_jitter = apply_jitter(capped, config.jitter, identity_seed);
    Duration::from_secs_f64(with_jitter).max(POLL_INTERVAL_FLOOR)
}

/// Apply ±jitter to an interval. `identity_seed` is XOR'd with the
/// time-based seed so that two triggers starting at the same instant
/// get different jitter values (thundering herd prevention).
fn apply_jitter(secs: f64, jitter_fraction: f64, identity_seed: u64) -> f64 {
    if jitter_fraction <= 0.0 {
        return secs;
    }
    let time_seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let seed = time_seed ^ identity_seed;
    let normalized = (seed % 1_000_000) as f64 / 1_000_000.0;
    let jitter = (normalized * 2.0 - 1.0) * jitter_fraction;
    (secs * (1.0 + jitter)).max(0.001)
}

/// Cheap hash of an action key for jitter identity seed.
fn action_key_seed(key: &nebula_core::ActionKey) -> u64 {
    let bytes = key.as_str().as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3); // FNV-1a prime
    }
    h
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
/// Consecutive empty poll results (no events) AND retryable errors
/// increase the sleep interval by `backoff_factor` up to
/// `max_interval`. A non-empty result resets to `base_interval`.
///
/// # Cursor checkpoint
///
/// The adapter unconditionally clones the cursor before calling
/// `poll()`. On any retryable error or dispatch failure (except
/// `DropAndContinue`), the cursor is restored from the snapshot.
/// This prevents cursor corruption from partial mutations inside
/// a failed `poll()` call.
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

        self.action.validate(ctx).await?;

        let config = self.action.poll_config();
        let identity_seed = action_key_seed(&self.action.metadata().key);
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
                .unwrap_or_else(|| compute_interval(&config, consecutive_empty, identity_seed));

            tokio::select! {
                () = ctx.cancellation.cancelled() => {
                    return Ok(());
                }
                () = tokio::time::sleep(interval) => {
                    // Always clone cursor before poll(). If poll() mutates
                    // cursor and then returns Err, or dispatch fails, we
                    // restore from this snapshot. The "don't mutate cursor
                    // on error" contract is unenforceable by the compiler,
                    // so the adapter defends against it unconditionally.
                    let saved_cursor = cursor.clone();

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
                                // Reset backoff when upstream produced data,
                                // regardless of dispatch outcome. Backoff
                                // protects against hammering an idle upstream,
                                // not against emitter failures. If the emitter
                                // is broken, polling less won't help — the
                                // upstream accumulates more events.
                                consecutive_empty = 0;
                                let ok = self.dispatch_events(
                                    &result.events,
                                    config.emit_failure,
                                    ctx,
                                ).await;

                                if !ok {
                                    match config.emit_failure {
                                        EmitFailurePolicy::DropAndContinue => {
                                            // All failures already skipped inside dispatch_events.
                                            // Cursor stays advanced — intentional for best-effort.
                                        }
                                        EmitFailurePolicy::RetryBatch => {
                                            cursor = saved_cursor;
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
                            cursor = saved_cursor;
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
