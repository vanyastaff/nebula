//! Keyed GCRA state behind a store contract.
//!
//! [`LimitStore`] is the seam a limit is enforced through: this crate ships
//! the in-process [`MemoryLimitStore`]; shared stores (a database, Redis)
//! live in storage crates and implement the same contract by running the
//! [`step`](super::step) functions atomically on their own clock. The
//! contract is here, not in a storage port, because this crate sits below
//! every storage crate and consumers of limits must not depend on storage.

use std::{collections::HashMap, fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use parking_lot::Mutex;
use tokio::time::Instant;

use super::{Denied, GcraState, Grant, Rate, nanos, step};

/// Longest accepted [`LimitKey`] in bytes.
pub const MAX_LIMIT_KEY_BYTES: usize = 512;

/// Opaque identity of one limit.
///
/// Callers namespace and, where it carries personal data, hash it before it
/// reaches a store: the store never interprets the key.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LimitKey(Arc<str>);

impl LimitKey {
    /// Validates a key: 1..=512 bytes of printable ASCII.
    ///
    /// # Errors
    ///
    /// [`LimitStoreError::InvalidKey`] otherwise.
    pub fn new(key: impl AsRef<str>) -> Result<Self, LimitStoreError> {
        let key = key.as_ref();
        let valid = !key.is_empty()
            && key.len() <= MAX_LIMIT_KEY_BYTES
            && key.bytes().all(|byte| byte.is_ascii_graphic());
        if valid {
            Ok(Self(Arc::from(key)))
        } else {
            Err(LimitStoreError::InvalidKey)
        }
    }

    /// The key as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LimitKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("LimitKey").field(&self.0).finish()
    }
}

/// Caller-chosen identity of one logical reservation.
///
/// Repeating a reservation with the same id (a retried request, a replayed
/// workflow step) returns the original grant instead of booking a second
/// slot, for as long as that grant's slot has not arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationId(pub u128);

/// One reservation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReserveRequest {
    /// Permits to book.
    pub permits: u32,
    /// Longest acceptable wait; [`Duration::MAX`] for none.
    pub max_wait: Duration,
    /// Makes a repeated request return the original grant.
    pub id: Option<ReservationId>,
}

impl ReserveRequest {
    /// Books `permits` with at most `max_wait` of waiting.
    #[must_use]
    pub const fn new(permits: u32, max_wait: Duration) -> Self {
        Self {
            permits,
            max_wait,
            id: None,
        }
    }

    /// Makes the request idempotent under `id`.
    #[must_use]
    pub const fn with_id(mut self, id: ReservationId) -> Self {
        self.id = Some(id);
        self
    }
}

/// Why a store could not answer. Distinct from a [`Denied`] decision: a
/// denial is an answer, this is its absence, which callers handle by policy
/// (fail closed, degrade).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LimitStoreError {
    /// The key is empty, too long, or not printable ASCII.
    #[error("limit key must be 1..=512 printable ASCII bytes")]
    InvalidKey,
    /// The backing store is unreachable or failed.
    #[error("limit store unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Store of keyed GCRA state.
///
/// Every method is one atomic transition of one key on the store's own
/// clock. Implementations must never read a caller's clock, and must apply
/// [`step::effective_rate`] so callers that disagree on a key's rate get the
/// stricter one. [`conformance`](super::conformance) holds the behaviour
/// every implementation is checked against.
pub trait LimitStore: Send + Sync {
    /// Books permits under `key` (see [`step::reserve`]).
    fn reserve(
        &self,
        key: &LimitKey,
        rate: &Rate,
        request: ReserveRequest,
    ) -> impl Future<Output = Result<Result<Grant, Denied>, LimitStoreError>> + Send;

    /// Blocks `key` for `retry_after`, capped at `max_penalty`
    /// (see [`step::penalize`]).
    fn penalize(
        &self,
        key: &LimitKey,
        rate: &Rate,
        retry_after: Duration,
        max_penalty: Duration,
    ) -> impl Future<Output = Result<(), LimitStoreError>> + Send;

    /// Returns `grant`'s permits if it is still the tail of `key`
    /// (see [`step::cancel`]); `false` when nothing was returned.
    ///
    /// The refund is `permits` intervals of the caller's `rate`, never of a
    /// stricter rate the key enforced meanwhile: the key's effective interval
    /// is at least the caller's, so refunding at the caller's never returns
    /// more than was booked.
    fn cancel(
        &self,
        key: &LimitKey,
        rate: &Rate,
        grant: &Grant,
    ) -> impl Future<Output = Result<bool, LimitStoreError>> + Send;
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe facade over [`LimitStore`], for `Arc<dyn ErasedLimitStore>`
/// injection.
pub trait ErasedLimitStore: Send + Sync {
    /// See [`LimitStore::reserve`].
    fn reserve_boxed<'a>(
        &'a self,
        key: &'a LimitKey,
        rate: &'a Rate,
        request: ReserveRequest,
    ) -> BoxFut<'a, Result<Result<Grant, Denied>, LimitStoreError>>;

    /// See [`LimitStore::penalize`].
    fn penalize_boxed<'a>(
        &'a self,
        key: &'a LimitKey,
        rate: &'a Rate,
        retry_after: Duration,
        max_penalty: Duration,
    ) -> BoxFut<'a, Result<(), LimitStoreError>>;

    /// See [`LimitStore::cancel`].
    fn cancel_boxed<'a>(
        &'a self,
        key: &'a LimitKey,
        rate: &'a Rate,
        grant: &'a Grant,
    ) -> BoxFut<'a, Result<bool, LimitStoreError>>;
}

impl<T: LimitStore> ErasedLimitStore for T {
    fn reserve_boxed<'a>(
        &'a self,
        key: &'a LimitKey,
        rate: &'a Rate,
        request: ReserveRequest,
    ) -> BoxFut<'a, Result<Result<Grant, Denied>, LimitStoreError>> {
        Box::pin(self.reserve(key, rate, request))
    }

    fn penalize_boxed<'a>(
        &'a self,
        key: &'a LimitKey,
        rate: &'a Rate,
        retry_after: Duration,
        max_penalty: Duration,
    ) -> BoxFut<'a, Result<(), LimitStoreError>> {
        Box::pin(self.penalize(key, rate, retry_after, max_penalty))
    }

    fn cancel_boxed<'a>(
        &'a self,
        key: &'a LimitKey,
        rate: &'a Rate,
        grant: &'a Grant,
    ) -> BoxFut<'a, Result<bool, LimitStoreError>> {
        Box::pin(self.cancel(key, rate, grant))
    }
}

#[derive(Debug, Default)]
struct Entry {
    state: GcraState,
    /// Rate the key enforced last (see [`step::effective_rate`]).
    rate: Option<Rate>,
    /// Grants still waiting for their slot, by reservation id.
    pending: HashMap<ReservationId, Grant>,
}

impl Entry {
    /// The rate to apply now, recorded as the key's rate.
    fn enforce(&mut self, now: u64, requested: &Rate) -> Rate {
        let rate = step::effective_rate(self.state, now, self.rate.as_ref(), requested);
        self.rate = Some(rate);
        rate
    }

    /// An entry with nothing pending and a past TAT carries no state: it
    /// behaves exactly like an absent key.
    fn is_idle(&self, now: u64) -> bool {
        self.pending.is_empty() && self.state.tat <= now
    }
}

/// Operations between sweeps of idle keys.
const SWEEP_EVERY: u64 = 1024;

#[derive(Debug, Default)]
struct Keys {
    entries: HashMap<LimitKey, Entry>,
    /// Shared by every key that arrived while the store was full.
    overflow: Entry,
    overflowed: u64,
    ops: u64,
}

/// Default bound on the keys a [`MemoryLimitStore`] holds.
pub const DEFAULT_MAX_KEYS: usize = 100_000;

/// In-process [`LimitStore`]: limits are per process.
///
/// Idle keys (TAT in the past, nothing pending) are swept every
/// 1024 operations; their state is indistinguishable from an absent key, so
/// sweeping never loosens a limit.
///
/// The number of keys is bounded, so callers that mint keys from request
/// data (a limit per chat, per recipient) cannot grow it without end. When
/// the store is full and sweeping frees nothing, a new key shares one
/// overflow limit with every other key that arrived meanwhile: stricter than
/// its own limit, never looser. [`overflowed`](Self::overflowed) counts
/// those operations.
#[derive(Debug)]
pub struct MemoryLimitStore {
    base: Instant,
    max_keys: usize,
    keys: Mutex<Keys>,
}

impl Default for MemoryLimitStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryLimitStore {
    /// An empty store holding up to [`DEFAULT_MAX_KEYS`] keys, whose clock
    /// starts now (`tokio::time`).
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_keys(DEFAULT_MAX_KEYS)
    }

    /// An empty store holding up to `max_keys` keys (at least one).
    #[must_use]
    pub fn with_max_keys(max_keys: usize) -> Self {
        Self {
            base: Instant::now(),
            max_keys: max_keys.max(1),
            keys: Mutex::new(Keys::default()),
        }
    }

    /// Keys currently holding state.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.lock().entries.len()
    }

    /// Operations that ran on the shared overflow limit because the store
    /// was full.
    #[must_use]
    pub fn overflowed(&self) -> u64 {
        self.keys.lock().overflowed
    }

    /// `true` when no key holds state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn now(&self) -> u64 {
        nanos(self.base.elapsed())
    }

    fn with_entry<T>(&self, key: &LimitKey, apply: impl FnOnce(&mut Entry, u64) -> T) -> T {
        let now = self.now();
        let mut guard = self.keys.lock();
        let keys = &mut *guard;
        keys.ops = keys.ops.wrapping_add(1);
        let is_new = !keys.entries.contains_key(key);
        // A new key into a full store sweeps first: idle keys make room.
        if keys.ops.is_multiple_of(SWEEP_EVERY) || (is_new && keys.entries.len() >= self.max_keys) {
            keys.entries.retain(|_, entry| !entry.is_idle(now));
        }
        if is_new && keys.entries.len() >= self.max_keys {
            keys.overflowed = keys.overflowed.saturating_add(1);
            let entry = &mut keys.overflow;
            entry.pending.retain(|_, grant| grant.allow_at > now);
            return apply(entry, now);
        }
        let entry = keys.entries.entry(key.clone()).or_default();
        entry.pending.retain(|_, grant| grant.allow_at > now);
        let result = apply(entry, now);
        if entry.is_idle(now) {
            keys.entries.remove(key);
        }
        drop(guard);
        result
    }
}

impl LimitStore for MemoryLimitStore {
    async fn reserve(
        &self,
        key: &LimitKey,
        rate: &Rate,
        request: ReserveRequest,
    ) -> Result<Result<Grant, Denied>, LimitStoreError> {
        Ok(self.with_entry(key, |entry, now| {
            if let Some(original) = request.id.and_then(|id| entry.pending.get(&id)) {
                return Ok(Grant {
                    wait: Duration::from_nanos(original.allow_at.saturating_sub(now)),
                    ..*original
                });
            }
            let rate = entry.enforce(now, rate);
            let (decision, next) =
                step::reserve(entry.state, now, &rate, request.permits, request.max_wait);
            if let Some(next) = next {
                entry.state = next;
            }
            if let (Ok(grant), Some(id)) = (&decision, request.id)
                && grant.allow_at > now
            {
                entry.pending.insert(id, *grant);
            }
            decision
        }))
    }

    async fn penalize(
        &self,
        key: &LimitKey,
        rate: &Rate,
        retry_after: Duration,
        max_penalty: Duration,
    ) -> Result<(), LimitStoreError> {
        self.with_entry(key, |entry, now| {
            let rate = entry.enforce(now, rate);
            entry.state = step::penalize(entry.state, now, &rate, retry_after, max_penalty);
        });
        Ok(())
    }

    async fn cancel(
        &self,
        key: &LimitKey,
        rate: &Rate,
        grant: &Grant,
    ) -> Result<bool, LimitStoreError> {
        Ok(self.with_entry(key, |entry, now| {
            match step::cancel(entry.state, now, rate, grant) {
                Some(next) => {
                    entry.state = next;
                    entry.pending.retain(|_, pending| pending.seq != grant.seq);
                    true
                },
                None => false,
            }
        }))
    }
}
