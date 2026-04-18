//! [`PollAction`] trait and [`PollTriggerAdapter`] — periodic pull-based triggers.
//!
//! A poll trigger runs a cancel-safe loop driven by [`PollConfig`]:
//! sleep → poll → emit events. The adapter owns all timing logic —
//! backoff, jitter, timeout, interval floor — so the action author
//! never writes a sleep loop.
//!
//! The cursor is in-memory only — across process restarts it resets to
//! the value returned by [`PollAction::initial_cursor`]. Cross-restart
//! persistence requires runtime storage integration (post-v1).

use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    hash::Hash,
    ops::{Deref, DerefMut},
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
/// # Responsibility — read before adding new fields
///
/// `PollConfig` declares the *integration-intrinsic timing
/// constraints* of a poll action: upper/lower interval bounds,
/// per-cycle timeout, failure policy. It does **not** decide *when*
/// polls are scheduled, staggered, or activated — those are runtime
/// concerns.
///
/// In particular, there is no "first poll delay" knob. The adapter
/// always runs the first `poll()` immediately after `start()`, and
/// the runtime is expected to delay its call to `start()` if it
/// needs per-integration warmup or fleet-wide stagger. If you find
/// yourself reaching for a `first_poll: FirstPoll` field, stop —
/// that responsibility belongs above this struct.
///
/// # Defaults
///
/// `PollConfig::default()` gives a fixed 60 s interval, no backoff, no
/// jitter, 30 s poll timeout, `DropAndContinue` on emit failure, and
/// no event cap.
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

    /// Suggested maximum pages per `poll()` call. The adapter does
    /// NOT enforce this — the action author checks it inside `poll()`
    /// and early-returns with [`PollResult::partial`] when the limit
    /// is reached. Exposed in config so it can be tuned without
    /// recompiling the action.
    pub max_pages_hint: Option<usize>,
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
            max_pages_hint: None,
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

    /// Set suggested maximum pages per poll cycle.
    #[must_use]
    pub fn with_max_pages_hint(mut self, max: usize) -> Self {
        self.max_pages_hint = Some(max);
        self
    }

    /// Clamp configuration to safe invariants and log a warn for
    /// every clamp that actually triggered.
    ///
    /// Called by [`PollTriggerAdapter`] once at `start()` before the
    /// loop. Rationale: configuration mistakes should degrade to
    /// sane defaults, not fail startup — credentials fail startup,
    /// wrong numbers do not.
    ///
    /// Clamps applied:
    /// - `max_interval >= base_interval` (otherwise `.min(max)` in `compute_interval` silently caps
    ///   below the requested base)
    /// - `backoff_factor ∈ [1.0, 60.0]` (guards `powi` against NaN / infinity / ridiculous values)
    /// - `poll_timeout > 0` (zero timeout makes every poll a retryable error)
    /// - `base_interval >= POLL_INTERVAL_FLOOR` (already clamped in `compute_interval`, warned here
    ///   for visibility)
    pub(crate) fn validate_and_clamp(
        &mut self,
        logger: &dyn crate::capability::ActionLogger,
        action_key: &nebula_core::ActionKey,
    ) {
        use crate::capability::ActionLogLevel;

        if self.base_interval < POLL_INTERVAL_FLOOR {
            logger.log(
                ActionLogLevel::Warn,
                &format!(
                    "poll trigger {action_key}: base_interval {:?} below floor {:?}; \
                     will be clamped to floor in compute_interval",
                    self.base_interval, POLL_INTERVAL_FLOOR,
                ),
            );
        }
        if self.max_interval < self.base_interval {
            logger.log(
                ActionLogLevel::Warn,
                &format!(
                    "poll trigger {action_key}: max_interval {:?} < base_interval {:?}; \
                     raising max_interval to base_interval",
                    self.max_interval, self.base_interval,
                ),
            );
            self.max_interval = self.base_interval;
        }
        if !self.backoff_factor.is_finite() || self.backoff_factor < 1.0 {
            logger.log(
                ActionLogLevel::Warn,
                &format!(
                    "poll trigger {action_key}: backoff_factor {} not in [1.0, 60.0]; \
                     clamping to 1.0 (no backoff)",
                    self.backoff_factor,
                ),
            );
            self.backoff_factor = 1.0;
        } else if self.backoff_factor > 60.0 {
            logger.log(
                ActionLogLevel::Warn,
                &format!(
                    "poll trigger {action_key}: backoff_factor {} exceeds 60.0; \
                     clamping to 60.0",
                    self.backoff_factor,
                ),
            );
            self.backoff_factor = 60.0;
        }
        if self.poll_timeout.is_zero() {
            logger.log(
                ActionLogLevel::Warn,
                &format!(
                    "poll trigger {action_key}: poll_timeout is zero; \
                     resetting to 30s default",
                ),
            );
            self.poll_timeout = Duration::from_secs(30);
        }
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
    /// **Always rolls back to pre-poll position**, even if
    /// [`PollCursor::checkpoint`] was called during the cycle. This
    /// means paginated polls re-fetch ALL pages (including
    /// successfully dispatched ones), not just the failed tail.
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

// ── PollOutcome + PollResult ───────────────────────────────────────────────

/// What happened during a poll cycle. Each variant has exactly one
/// semantic — no optional-field matrix, no invalid combinations.
///
/// The fourth state ("total failure, no events") is represented by
/// returning `Err` from `poll()` directly.
#[derive(Debug)]
#[non_exhaustive]
pub enum PollOutcome<E> {
    /// No new events. Cursor may have advanced (e.g., heartbeat pages
    /// consumed without producing events). Adapter keeps cursor as-is,
    /// applies backoff.
    Idle,

    /// Events ready to dispatch. Adapter emits all, advances cursor,
    /// resets backoff.
    Ready {
        /// Events to emit as workflow executions.
        events: Vec<E>,
    },

    /// Partial progress — some events collected before an error
    /// (e.g., pagination failed mid-way). Adapter dispatches the
    /// events, then rolls back cursor to the last
    /// [`PollCursor::checkpoint`] and applies backoff.
    ///
    /// For total failure (no events collected), return `Err` from
    /// `poll()` instead — it triggers a full pre-poll rollback.
    Partial {
        /// Events collected before the error.
        events: Vec<E>,
        /// The error that stopped the poll cycle.
        error: ActionError,
    },
}

/// What [`PollAction::poll`] returns — outcome + optional interval
/// override.
///
/// `override_next` is orthogonal to outcome: a Stripe `Retry-After`
/// header applies whether the cycle was `Ready`, `Partial`, or `Idle`.
#[derive(Debug)]
#[non_exhaustive]
pub struct PollResult<E> {
    /// What happened during this poll cycle.
    pub outcome: PollOutcome<E>,
    /// One-shot override for the next sleep interval. Useful when the
    /// upstream API provides a `Retry-After` or backoff hint. `None`
    /// means use the normal computed interval.
    pub override_next: Option<Duration>,
}

impl<E> PollResult<E> {
    /// Build a [`PollResult`] from an outcome with no override.
    ///
    /// The raw struct is `#[non_exhaustive]`; use this constructor
    /// (plus chained setters like [`with_override_next`](Self::with_override_next))
    /// instead of field literals outside the crate.
    #[must_use]
    pub fn from_outcome(outcome: PollOutcome<E>) -> Self {
        Self {
            outcome,
            override_next: None,
        }
    }

    /// Set a one-shot interval override for the next cycle.
    #[must_use]
    pub fn with_override_next(mut self, interval: Duration) -> Self {
        self.override_next = Some(interval);
        self
    }

    /// Create a partial result: events from successful pages + the
    /// error from the failed page. Adapter dispatches the events and
    /// rolls cursor back to the last checkpoint.
    ///
    /// # Panics
    ///
    /// Debug-asserts that `events` is non-empty. If you have no events,
    /// return `Err` from `poll()` for a full pre-poll rollback.
    pub fn partial(events: Vec<E>, error: ActionError) -> Self {
        debug_assert!(
            !events.is_empty(),
            "PollResult::partial requires events; use Err from poll() for total failure"
        );
        Self {
            outcome: PollOutcome::Partial { events, error },
            override_next: None,
        }
    }
}

impl<E> From<Vec<E>> for PollResult<E> {
    fn from(events: Vec<E>) -> Self {
        Self {
            outcome: if events.is_empty() {
                PollOutcome::Idle
            } else {
                PollOutcome::Ready { events }
            },
            override_next: None,
        }
    }
}

// ── PollHealth ────────────────────────────────────────────────────────────

/// Poll-specific health details. Serialized into
/// [`TriggerHealth`](crate::capability::TriggerHealth) by the adapter.
#[derive(Debug, Clone, Serialize)]
// Removed: PollHealth moved to atomic-based TriggerHealth in capability.rs.

// ── PollCursor ────────────────────────────────────────────────────────────

/// Cursor wrapper with checkpoint capability for incremental progress
/// inside a single `poll()` call.
///
/// The adapter creates a `PollCursor` before each poll cycle. Actions
/// that fetch paginated data call [`checkpoint`](Self::checkpoint)
/// after each successfully processed page. On error, the adapter
/// rolls back to the last checkpoint — not to the pre-poll position
/// — so completed pages are not re-fetched.
///
/// For non-paginated actions, `PollCursor` is transparent: `Deref`
/// and `DerefMut` delegate to the inner cursor, and no checkpoint
/// calls are needed. On error, the adapter does a full rollback to
/// the pre-poll position (since no checkpoints were made, checkpoint
/// equals the initial position).
///
/// # Example — paginated Gmail poll
///
/// ```rust,ignore
/// async fn poll(&self, cursor: &mut PollCursor<GmailCursor>, ctx: &TriggerContext)
///     -> Result<PollResult<GmailEvent>, ActionError>
/// {
///     let mut all_events = Vec::new();
///     let mut page_token = None;
///
///     loop {
///         let page = match self.client.history_list(cursor.history_id, page_token).await {
///             Ok(p) => p,
///             Err(e) => return Ok(PollResult::partial(all_events, e.into())),
///         };
///
///         all_events.extend(page.events);
///         cursor.history_id = page.last_history_id;
///         cursor.checkpoint(); // safe to resume from here
///
///         match page.next_page_token {
///             Some(token) => page_token = Some(token),
///             None => break,
///         }
///     }
///
///     Ok(all_events.into())
/// }
/// ```
pub struct PollCursor<C> {
    current: C,
    checkpoint: C,
}

impl<C: Clone> PollCursor<C> {
    /// Create a new poll cursor. The checkpoint is initialized to a
    /// clone of the current position.
    ///
    /// The adapter creates this automatically; action authors use it
    /// in unit tests to construct a cursor for direct `poll()` calls.
    pub fn new(cursor: C) -> Self {
        let checkpoint = cursor.clone();
        Self {
            current: cursor,
            checkpoint,
        }
    }

    /// Save the current position as safe-to-resume-from.
    ///
    /// Call after each successfully processed page. If `poll()`
    /// returns [`PollResult::partial`] after this, the adapter
    /// resumes from this checkpoint instead of re-fetching all pages.
    pub fn checkpoint(&mut self) {
        self.checkpoint = self.current.clone();
    }

    /// Roll back to the last checkpoint (or initial position if
    /// no checkpoints were made).
    pub(crate) fn rollback(&mut self) {
        self.current = self.checkpoint.clone();
    }

    /// Consume and return the current cursor position.
    pub(crate) fn into_current(self) -> C {
        self.current
    }
}

impl<C> Deref for PollCursor<C> {
    type Target = C;
    fn deref(&self) -> &C {
        &self.current
    }
}

impl<C> DerefMut for PollCursor<C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.current
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
/// async fn poll(&self, cursor: &mut PollCursor<Cursor>, ctx: &TriggerContext)
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
/// # Sizing `max_seen`
///
/// **Size it for at least twice the maximum batch you expect per
/// poll cycle.** FIFO eviction happens inside `filter_new` as items
/// are inserted — if a single batch exceeds `max_seen`, entries from
/// the *start* of the same batch are evicted before the batch
/// finishes. On the next poll those boundary events re-enter as
/// "new" and slip through dedup. Example: Stripe polled every 60 s
/// with 6 000 events per cycle needs `max_seen ≥ 12 000`, not the
/// default 1 000.
///
/// `filter_new` `debug_assert!`s when a single call exceeds the cap,
/// so tests catch this in development.
///
/// # State growth
///
/// The seen set is capped at `max_seen` (default 1,000). When the
/// cap is exceeded, the oldest entries are evicted (FIFO). Downstream
/// consumers should be idempotent as a defense-in-depth measure.
///
/// The entire cursor (including the seen set) is cloned by the
/// adapter on each cycle for rollback safety (pre-poll snapshot +
/// internal checkpoint). Keep `K` lightweight — prefer `String` or
/// integer IDs over large composite keys. For integrations that need
/// more than 10k dedup keys, consider a custom cursor with
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
        // Normalize: rebuild both structures from order,
        // deduplicating and enforcing max_seen cap. Cap is clamped to
        // >= 1 — `max_seen = 0` would leave try_insert in an add-then-
        // immediately-evict loop and silently disable dedup.
        let cap = wire.max_seen.max(1);
        let mut lookup = HashSet::with_capacity(wire.seen.len().min(cap));
        let mut order = VecDeque::with_capacity(wire.seen.len().min(cap));
        for key in wire.seen {
            if lookup.insert(key.clone()) {
                order.push_back(key);
            }
        }
        while lookup.len() > cap {
            if let Some(old) = order.pop_front() {
                lookup.remove(&old);
            }
        }

        Ok(Self {
            inner: wire.inner,
            order,
            lookup,
            max_seen: cap,
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
    ///
    /// # Panics (debug only)
    ///
    /// Debug-asserts that `items.len() <= max_seen`. A batch larger
    /// than the cap means FIFO eviction will drop items from the
    /// start of the same batch — duplicates will leak across cycles.
    /// See the `DeduplicatingCursor` type docs for sizing guidance.
    pub fn filter_new<T>(&mut self, items: Vec<T>, key_fn: impl Fn(&T) -> K) -> Vec<T> {
        debug_assert!(
            items.len() <= self.max_seen,
            "DeduplicatingCursor::filter_new: batch of {} exceeds max_seen {} — \
             dedup will leak at the boundary. Size max_seen for ≥ 2 × max batch.",
            items.len(),
            self.max_seen,
        );
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
/// poll → dispatch → sleep. The first cycle runs immediately on
/// start. Cancellation via `TriggerContext::cancellation`.
///
/// # Persistence — READ BEFORE SHIPPING
///
/// **Cursor state is in-memory only.** On process restart,
/// [`initial_cursor`](Self::initial_cursor) is called again and all
/// progress accumulated between the last successful poll and the
/// restart is discarded.
///
/// This is acceptable for **best-effort** integrations:
/// - RSS / Atom / news feeds
/// - Dashboards, metrics, health checks
/// - Any integration where occasional loss or replay is cheap
///
/// This is **NOT acceptable** for:
/// - Payments (Stripe events, invoice sync, ledger updates)
/// - Audit / compliance feeds
/// - CRM sync where "missed lead = lost revenue"
/// - Any integration with strong delivery expectations
///
/// For the NOT-acceptable class, a process restart will either:
/// 1. Re-flood upstream and re-emit every historical event (if `initial_cursor` returns
///    `Default::default()`), or
/// 2. Silently skip every event that arrived between the last successful poll and the restart (if
///    `initial_cursor` returns "now"). **No error. No warning. No health degradation.**
///
/// Durable cursor storage across restarts is tracked as future
/// work — the runtime will grow a `TriggerStateStore` capability
/// that persists cursor snapshots. Until that lands, do **not**
/// ship a high-value integration against this trait unless your
/// downstream workflow is fully idempotent (every event carries an
/// external dedup id and the downstream rejects duplicates).
///
/// # Cursor lifecycle
///
/// 1. [`initial_cursor`](Self::initial_cursor) — called once at start; default returns
///    `Default::default()`. Override to start from "now" instead of "beginning of time" (e.g.,
///    latest Gmail history ID, current Stripe event cursor).
/// 2. Each cycle: adapter wraps cursor in [`PollCursor`], calls `poll()`.
/// 3. [`PollOutcome::Ready`]: cursor advances to final position.
/// 4. [`PollOutcome::Partial`]: events dispatched, cursor rolls back to last
///    [`PollCursor::checkpoint`].
/// 5. [`PollOutcome::Idle`]: cursor kept as-is (may have advanced past empty pages).
/// 6. `Err` from `poll()`: cursor rolls back to pre-poll position.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::poll::{PollAction, PollConfig, PollCursor, PollResult};
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
///     async fn poll(&self, cursor: &mut PollCursor<String>, ctx: &TriggerContext)
///         -> Result<PollResult<serde_json::Value>, ActionError> {
///         let items = fetch_rss(&self.feed_url, &*cursor).await?;
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

    /// Initial cursor position on first start.
    ///
    /// Called once before the first poll cycle. Override to start from
    /// "now" (e.g., latest upstream ID) instead of `Default::default()`
    /// (beginning of time). This prevents the first-run flood that
    /// occurs when `Default::default()` means "fetch everything."
    ///
    /// Default: returns `Self::Cursor::default()`.
    fn initial_cursor(
        &self,
        _ctx: &TriggerContext,
    ) -> impl Future<Output = Result<Self::Cursor, ActionError>> + Send {
        async { Ok(Self::Cursor::default()) }
    }

    /// Execute one poll cycle.
    ///
    /// Receives a [`PollCursor`] wrapping the inner cursor. Use
    /// `Deref`/`DerefMut` to access the cursor transparently. For
    /// paginated APIs, call [`PollCursor::checkpoint`] after each
    /// successful page to enable incremental progress.
    ///
    /// Return one of three outcomes via [`PollResult`]:
    ///
    /// - `Ok(events.into())` — [`PollOutcome::Ready`] (non-empty) or [`PollOutcome::Idle`] (empty).
    ///   Adapter advances cursor.
    /// - `Ok(PollResult::partial(events, error))` — events from successful pages + the error.
    ///   Adapter dispatches events, rolls back to last checkpoint.
    /// - `Err(e)` — total failure. Adapter rolls back to pre-poll.
    ///
    /// # Errors
    ///
    /// [`ActionError::Retryable`] for transient failures (skip this cycle),
    /// [`ActionError::Fatal`] to stop the trigger permanently.
    fn poll(
        &self,
        cursor: &mut PollCursor<Self::Cursor>,
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
            },
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

/// Cheap FNV-1a hash over action key + workflow id + trigger id.
///
/// Each running trigger instance gets a unique seed, so fleet
/// deployments of the same action type no longer produce correlated
/// jitter (thundering herd). Action type alone is insufficient —
/// see commit "poll: jitter seed uses trigger identity".
fn trigger_seed(
    action_key: &nebula_core::ActionKey,
    workflow_id: &nebula_core::WorkflowId,
    trigger_id: &nebula_core::NodeKey,
) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    let mix = |h: &mut u64, bytes: &[u8]| {
        for &b in bytes {
            *h ^= u64::from(b);
            *h = h.wrapping_mul(0x0100_0000_01b3); // FNV-1a prime
        }
    };
    mix(&mut h, action_key.as_str().as_bytes());
    mix(&mut h, &workflow_id.get().to_bytes());
    mix(&mut h, trigger_id.as_str().as_bytes());
    h
}

/// Result of dispatching a batch of events.
enum DispatchResult {
    /// All events dispatched (or dropped under `DropAndContinue`).
    Ok { emitted: usize, dropped: usize },
    /// Hard failure — stopped at the first non-droppable error.
    Failed,
}

impl DispatchResult {
    fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    /// True if events were produced by the action but none reached
    /// the emitter — total silent loss under `DropAndContinue`.
    fn is_total_loss(&self) -> bool {
        match self {
            Self::Ok {
                emitted, dropped, ..
            } => *emitted == 0 && *dropped > 0,
            _ => false,
        }
    }

    fn emitted(&self) -> usize {
        match self {
            Self::Ok { emitted, .. } => *emitted,
            Self::Failed => 0,
        }
    }
}

/// Outcome of a resolved poll cycle — drives cursor and backoff in the
/// main loop.
struct CycleOutcome<C> {
    /// New cursor position to store.
    cursor: C,
    /// Whether to increment `consecutive_empty` (triggers backoff).
    backoff: bool,
    /// Whether this cycle counts as a health error (dispatch failure,
    /// total drop, or retryable poll error surfaced through Partial).
    /// Independent of `backoff`: a cycle can both back off and error.
    errored: bool,
    /// One-shot interval override for the next cycle (e.g., Retry-After).
    override_next: Option<Duration>,
    /// Number of events successfully emitted this cycle.
    emitted: usize,
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
/// # Cursor rollback strategy
///
/// Rollback target depends on [`PollOutcome`] variant:
///
/// - **`Idle`**: cursor = current position (may have advanced past empty pages).
/// - **`Ready`** (dispatch ok): cursor = final position.
/// - **`Ready`** (dispatch fail + `RetryBatch`): cursor = pre-poll.
/// - **`Partial`**: cursor = last [`PollCursor::checkpoint`]. Events dispatched before rollback.
/// - **`Err`**: cursor = pre-poll position. No events dispatched.
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

    /// Resolve a successful poll cycle: dispatch events, determine cursor
    /// fate and backoff. Each `PollOutcome` variant maps to one code path
    /// — no optional-field matrix.
    async fn resolve_cycle(
        &self,
        result: PollResult<A::Event>,
        mut poll_cursor: PollCursor<A::Cursor>,
        pre_poll: A::Cursor,
        config: &PollConfig,
        ctx: &TriggerContext,
    ) -> Result<CycleOutcome<A::Cursor>, ActionError>
    where
        A::Event: Send + Sync,
    {
        let override_next = result.override_next;

        match result.outcome {
            PollOutcome::Idle => Ok(CycleOutcome {
                cursor: poll_cursor.into_current(),
                backoff: true,
                errored: false,
                override_next,
                emitted: 0,
            }),

            PollOutcome::Ready { events } => {
                let dr = self
                    .dispatch_events(&events, config.emit_failure, ctx)
                    .await;
                let emitted = dr.emitted();
                if dr.is_ok() {
                    // Total loss under DropAndContinue: all events
                    // dropped (serialize/emit failed for every one).
                    // Cursor advances, but health must record an error
                    // — the dashboard must not see "idle" while 100 %
                    // of events are being silently lost.
                    let total_loss = dr.is_total_loss();
                    Ok(CycleOutcome {
                        cursor: poll_cursor.into_current(),
                        backoff: total_loss,
                        errored: total_loss,
                        override_next,
                        emitted,
                    })
                } else {
                    self.handle_dispatch_failure(poll_cursor, pre_poll, config, override_next)
                }
            },

            PollOutcome::Partial { events, error } => {
                if events.is_empty() {
                    // Empty-events Partial is the same shape as a
                    // top-level Err from poll(): roll back, log, back
                    // off, report health error for retryable; bubble
                    // fatal.
                    poll_cursor.rollback();
                    if error.is_fatal() {
                        return Err(error);
                    }
                    if self.poll_warn.should_log() {
                        ctx.logger.log(
                            ActionLogLevel::Warn,
                            &format!(
                                "poll trigger {}: partial with no events, \
                                 retryable error: {error}",
                                self.action.metadata().base.key,
                            ),
                        );
                    }
                    return Ok(CycleOutcome {
                        cursor: poll_cursor.into_current(),
                        backoff: true,
                        errored: true,
                        override_next,
                        emitted: 0,
                    });
                }

                let event_count = events.len();
                let dr = self
                    .dispatch_events(&events, config.emit_failure, ctx)
                    .await;
                let emitted = dr.emitted();

                poll_cursor.rollback();

                if !dr.is_ok() {
                    match config.emit_failure {
                        EmitFailurePolicy::DropAndContinue => {},
                        EmitFailurePolicy::RetryBatch => {
                            return Ok(CycleOutcome {
                                cursor: pre_poll,
                                backoff: true,
                                errored: true,
                                override_next,
                                emitted,
                            });
                        },
                        EmitFailurePolicy::StopTrigger => {
                            return Err(ActionError::fatal(format!(
                                "poll trigger {}: dispatch failed, \
                                 stopping (StopTrigger policy)",
                                self.action.metadata().base.key,
                            )));
                        },
                    }
                }

                if error.is_fatal() {
                    return Err(error);
                }

                if self.poll_warn.should_log() {
                    ctx.logger.log(
                        ActionLogLevel::Warn,
                        &format!(
                            "poll trigger {}: partial error \
                             after dispatching {event_count} events: {error}",
                            self.action.metadata().base.key,
                        ),
                    );
                }

                Ok(CycleOutcome {
                    cursor: poll_cursor.into_current(),
                    backoff: true,
                    errored: true,
                    override_next,
                    emitted,
                })
            },
        }
    }

    /// Handle dispatch failure for `Ready` outcome — cursor fate
    /// depends on `EmitFailurePolicy`.
    fn handle_dispatch_failure(
        &self,
        poll_cursor: PollCursor<A::Cursor>,
        pre_poll: A::Cursor,
        config: &PollConfig,
        override_next: Option<Duration>,
    ) -> Result<CycleOutcome<A::Cursor>, ActionError> {
        match config.emit_failure {
            EmitFailurePolicy::DropAndContinue => {
                // Unreachable: DropAndContinue never triggers
                // dispatch failure (dispatch_events returns Ok).
                // Defensive: advance cursor, record error.
                Ok(CycleOutcome {
                    cursor: poll_cursor.into_current(),
                    backoff: true,
                    errored: true,
                    override_next,
                    emitted: 0,
                })
            },
            EmitFailurePolicy::RetryBatch => Ok(CycleOutcome {
                cursor: pre_poll,
                backoff: true,
                errored: true,
                override_next,
                emitted: 0,
            }),
            EmitFailurePolicy::StopTrigger => Err(ActionError::fatal(format!(
                "poll trigger {}: dispatch failed, stopping (StopTrigger policy)",
                self.action.metadata().base.key,
            ))),
        }
    }

    async fn dispatch_events(
        &self,
        events: &[A::Event],
        policy: EmitFailurePolicy,
        ctx: &TriggerContext,
    ) -> DispatchResult
    where
        A::Event: Send + Sync,
    {
        let action_key = &self.action.metadata().base.key;
        let mut emitted: usize = 0;
        let mut dropped: usize = 0;
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
                        dropped += 1;
                        continue;
                    }
                    return DispatchResult::Failed;
                },
            };
            if let Err(e) = ctx.emitter.emit(payload).await {
                if self.emit_warn.should_log() {
                    ctx.logger.log(
                        ActionLogLevel::Warn,
                        &format!("poll trigger {action_key}: emitter.emit failed: {e}"),
                    );
                }
                if policy == EmitFailurePolicy::DropAndContinue {
                    dropped += 1;
                    continue;
                }
                return DispatchResult::Failed;
            }
            emitted += 1;
        }
        DispatchResult::Ok { emitted, dropped }
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
                "poll trigger already started; call stop() and await the task before start() again",
            ));
        }
        let _guard = StartedGuard(&self.started);

        self.action.validate(ctx).await?;

        let action_key = self.action.metadata().base.key.clone();
        let mut config = self.action.poll_config();
        config.validate_and_clamp(&*ctx.logger, &action_key);

        let identity_seed = trigger_seed(&action_key, &ctx.workflow_id, &ctx.trigger_id);
        let mut cursor = self.action.initial_cursor(ctx).await?;
        let mut consecutive_empty: u32 = 0;
        let mut override_next: Option<Duration> = None;

        // Effective max_interval for override clamping: never below
        // the global floor, regardless of a sloppy PollConfig.
        let effective_max_interval = config.max_interval.max(POLL_INTERVAL_FLOOR);

        loop {
            // H1: pre-poll cancellation check. This is the only
            // place a newly-activated-then-immediately-cancelled
            // trigger can exit before running any poll. Cheap
            // atomic read, no futures, no await.
            if ctx.cancellation.is_cancelled() {
                return Ok(());
            }

            // Poll + dispatch first, sleep after. Matches
            // scheduler-style expectations: PollConfig::fixed(60s)
            // means "every 60s starting now", not "wait 60s and
            // then every 60s". Pre-activation delay is a runtime
            // concern: runtime delays its call to start() if it
            // wants fleet stagger or warmup.
            let pre_poll = cursor.clone();
            let mut poll_cursor = PollCursor::new(cursor);

            let poll_result = match tokio::time::timeout(
                config.poll_timeout,
                self.action.poll(&mut poll_cursor, ctx),
            )
            .await
            {
                Ok(result) => result,
                Err(_elapsed) => Err(ActionError::retryable(format!(
                    "poll trigger {action_key}: poll() timed out after {:?}",
                    config.poll_timeout,
                ))),
            };

            match poll_result {
                Ok(result) => {
                    let outcome = self
                        .resolve_cycle(result, poll_cursor, pre_poll, &config, ctx)
                        .await?;
                    cursor = outcome.cursor;
                    override_next = outcome.override_next;
                    if outcome.backoff {
                        consecutive_empty = consecutive_empty.saturating_add(1);
                    } else {
                        consecutive_empty = 0;
                    }
                    // Health reporting is orthogonal to backoff
                    // and cursor fate: errored > success > idle.
                    if outcome.errored {
                        ctx.health.record_error();
                    } else if outcome.emitted > 0 {
                        ctx.health.record_success(outcome.emitted as u64);
                    } else {
                        ctx.health.record_idle();
                    }
                },
                Err(e) if e.is_fatal() => return Err(e),
                Err(e) => {
                    cursor = pre_poll;
                    consecutive_empty = consecutive_empty.saturating_add(1);
                    ctx.health.record_error();
                    if self.poll_warn.should_log() {
                        ctx.logger.log(
                            ActionLogLevel::Warn,
                            &format!(
                                "poll trigger {action_key}: retryable poll error, \
                                 will retry next cycle: {e}"
                            ),
                        );
                    }
                },
            }

            // Sleep with cancellation. The sleep interval is
            // computed AFTER the poll so override_next (e.g.
            // Retry-After from upstream) from the cycle we just
            // ran takes effect.
            let interval = override_next
                .take()
                .map(|d| d.clamp(POLL_INTERVAL_FLOOR, effective_max_interval))
                .unwrap_or_else(|| compute_interval(&config, consecutive_empty, identity_seed));

            tokio::select! {
                () = ctx.cancellation.cancelled() => return Ok(()),
                () = tokio::time::sleep(interval) => {}
            }
        }
    }

    /// Initiate shutdown by cancelling `ctx.cancellation`.
    ///
    /// [`PollTriggerAdapter::start`] runs an internal loop that exits
    /// when the cancellation token fires. This method fires the
    /// token itself — previously it was a no-op and relied on the
    /// caller to cancel the token, which silently broke the
    /// `stop() -> start()` restart pattern when callers forgot.
    ///
    /// **This does not wait for the loop to finish.** The
    /// `StartedGuard` RAII pattern clears the `started` flag only
    /// after the background task returns from `start()`. Callers
    /// needing synchronous restart semantics must:
    ///
    /// ```text
    /// adapter.stop(&ctx).await?;
    /// handle.await??;           // wait for the spawned start() task
    /// adapter.start(&ctx).await?;
    /// ```
    ///
    /// Without the `handle.await`, the second `start()` races the
    /// first task's exit and may return `Fatal("already started")`.
    /// A future runtime-owned task handle (tracked as F-level work)
    /// will hide this footgun.
    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        ctx.cancellation.cancel();
        Ok(())
    }
}

impl<A: PollAction> std::fmt::Debug for PollTriggerAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollTriggerAdapter")
            .field("action", &self.action.metadata().base.key)
            .finish_non_exhaustive()
    }
}
