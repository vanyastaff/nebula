//! Bounded cache of live [`CredentialHandle`]s.
//!
//! The resolver keeps one handle per `(selector, scheme TypeId)` so a refresh can
//! [`CredentialHandle::replace`] material in place for every holder. Each entry owns
//! decrypted scheme material, so the cache is bounded instead of growing for the
//! process lifetime:
//!
//! - an entry not resolved for [`DEFAULT_IDLE_TTL`] is dropped by an amortized sweep
//!   that runs at most once per TTL, on the resolve path;
//! - inserting past [`DEFAULT_MAX_ENTRIES`] first sweeps idle entries, then drops the
//!   least recently used one;
//! - delete, revoke, and a resolve that finds the row absent or tombstoned drop the
//!   selector's entries at once ([`HandleCache::evict_selector`]).
//!
//! A hit costs one shard read lock plus a relaxed atomic store of the access time.
//! An evicted handle is not updated in place by later refreshes. The next resolve
//! returns a new handle, so a holder that has not resolved within the TTL sees its
//! old snapshot until it resolves again.

use std::{
    any::{Any, TypeId},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;

use crate::{AuthScheme, CredentialHandle, CredentialSelector};

/// Idle time after which a cached handle and its decrypted material are dropped.
pub(super) const DEFAULT_IDLE_TTL: Duration = Duration::from_mins(15);

/// Maximum number of cached handles held at once.
pub(super) const DEFAULT_MAX_ENTRIES: usize = 10_000;

type HandleKey = (CredentialSelector, TypeId);

struct CachedHandle {
    handle: Arc<dyn Any + Send + Sync>,
    /// Milliseconds since [`HandleCache::origin`] of the last hit or insert.
    last_used_ms: AtomicU64,
}

/// Bounded `(selector, scheme TypeId)` → [`CredentialHandle`] cache.
///
/// Read-heavy, write-light: every resolve reads; writes happen on a miss, an
/// eviction, or a sweep. `DashMap` sharding keeps a write to one shard from
/// blocking reads on the others.
pub(super) struct HandleCache {
    entries: DashMap<HandleKey, CachedHandle>,
    origin: Instant,
    idle_ttl: Duration,
    max_entries: usize,
    /// Earliest [`Self::origin`]-relative millisecond at which the next idle sweep runs.
    next_sweep_ms: AtomicU64,
}

impl Default for HandleCache {
    fn default() -> Self {
        Self::new(DEFAULT_IDLE_TTL, DEFAULT_MAX_ENTRIES)
    }
}

impl HandleCache {
    pub(super) fn new(idle_ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: DashMap::new(),
            origin: Instant::now(),
            idle_ttl,
            max_entries: max_entries.max(1),
            next_sweep_ms: AtomicU64::new(duration_ms(idle_ttl)),
        }
    }

    fn key<S: AuthScheme>(selector: &CredentialSelector) -> HandleKey {
        (selector.clone(), TypeId::of::<S>())
    }

    fn millis_at(&self, now: Instant) -> u64 {
        duration_ms(now.saturating_duration_since(self.origin))
    }

    fn is_idle(&self, last_used_ms: u64, now_ms: u64) -> bool {
        now_ms.saturating_sub(last_used_ms) >= duration_ms(self.idle_ttl)
    }

    /// The cached handle for `selector`, marking it used at `now`.
    pub(super) fn get<S: AuthScheme>(
        &self,
        selector: &CredentialSelector,
        now: Instant,
    ) -> Option<CredentialHandle<S>> {
        let entry = self.entries.get(&Self::key::<S>(selector))?;
        entry
            .last_used_ms
            .store(self.millis_at(now), Ordering::Relaxed);
        let handle = Arc::clone(&entry.handle);
        drop(entry);
        handle
            .downcast::<CredentialHandle<S>>()
            .ok()
            .map(|arc| (*arc).clone())
    }

    /// Cache `handle` for `selector`, keeping the cache within its entry bound.
    pub(super) fn insert<S: AuthScheme>(
        &self,
        selector: &CredentialSelector,
        handle: CredentialHandle<S>,
        now: Instant,
    ) {
        let key = Self::key::<S>(selector);
        let now_ms = self.millis_at(now);
        if !self.entries.contains_key(&key) && self.entries.len() >= self.max_entries {
            self.evict_idle(now_ms);
            while self.entries.len() >= self.max_entries {
                if !self.evict_least_recently_used() {
                    break;
                }
            }
        }
        self.entries.insert(
            key,
            CachedHandle {
                handle: Arc::new(handle),
                last_used_ms: AtomicU64::new(now_ms),
            },
        );
    }

    /// Drop every handle cached for `selector`, whatever its scheme type.
    pub(super) fn evict_selector(&self, selector: &CredentialSelector) {
        self.entries.retain(|(cached, _), _| cached != selector);
    }

    /// Drop idle handles if the sweep interval has elapsed since the last sweep.
    ///
    /// One relaxed load on the common path; only the caller that wins the
    /// compare-exchange sweeps.
    pub(super) fn sweep_if_due(&self, now: Instant) {
        let now_ms = self.millis_at(now);
        let due = self.next_sweep_ms.load(Ordering::Relaxed);
        if now_ms < due {
            return;
        }
        let next = now_ms.saturating_add(duration_ms(self.idle_ttl));
        if self
            .next_sweep_ms
            .compare_exchange(due, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.evict_idle(now_ms);
        }
    }

    fn evict_idle(&self, now_ms: u64) {
        let before = self.entries.len();
        self.entries
            .retain(|_, cached| !self.is_idle(cached.last_used_ms.load(Ordering::Relaxed), now_ms));
        let evicted = before.saturating_sub(self.entries.len());
        if evicted > 0 {
            tracing::debug!(
                evicted,
                remaining = self.entries.len(),
                "evicted idle credential handles"
            );
        }
    }

    /// Remove the entry with the oldest access time; `false` when empty.
    fn evict_least_recently_used(&self) -> bool {
        let oldest = self
            .entries
            .iter()
            .min_by_key(|entry| entry.value().last_used_ms.load(Ordering::Relaxed))
            .map(|entry| entry.key().clone());
        match oldest {
            Some(key) => {
                self.entries.remove(&key);
                tracing::debug!("evicted least recently used credential handle at capacity");
                true
            },
            None => false,
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use nebula_storage_port::CredentialOwner;

    use super::*;
    use crate::{CredentialId, SecretString, scheme::SecretToken};

    fn selector() -> CredentialSelector {
        CredentialSelector::new(
            CredentialOwner::from_canonical("cache-owner"),
            CredentialId::new(),
        )
    }

    fn handle(secret: &str) -> CredentialHandle<SecretToken> {
        CredentialHandle::new(SecretToken::new(SecretString::new(secret)), "cred")
    }

    const TTL: Duration = Duration::from_mins(1);

    #[test]
    fn hit_returns_the_same_live_handle() {
        let cache = HandleCache::new(TTL, 8);
        let now = Instant::now();
        let sel = selector();
        let original = handle("a");
        cache.insert(&sel, original.clone(), now);

        let cached = cache.get::<SecretToken>(&sel, now).expect("cached");
        cached.replace(SecretToken::new(SecretString::new("b")));
        assert_eq!(original.snapshot().token().expose_secret(), "b");
    }

    #[test]
    fn idle_handles_are_swept_and_recently_used_ones_kept() {
        let cache = HandleCache::new(TTL, 8);
        let start = Instant::now();
        let idle = selector();
        let busy = selector();
        cache.insert(&idle, handle("idle"), start);
        cache.insert(&busy, handle("busy"), start);

        // `busy` is resolved halfway through the TTL; `idle` never again.
        let half = start + TTL / 2;
        assert!(cache.get::<SecretToken>(&busy, half).is_some());

        cache.sweep_if_due(start + TTL + Duration::from_secs(1));
        assert!(cache.get::<SecretToken>(&idle, start + TTL).is_none());
        assert!(cache.get::<SecretToken>(&busy, start + TTL).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn sweep_does_not_run_before_it_is_due() {
        let cache = HandleCache::new(TTL, 8);
        let start = Instant::now();
        let sel = selector();
        cache.insert(&sel, handle("a"), start);

        cache.sweep_if_due(start + TTL / 2);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn insert_at_capacity_evicts_the_least_recently_used_handle() {
        let cache = HandleCache::new(TTL, 2);
        let start = Instant::now();
        let first = selector();
        let second = selector();
        let third = selector();
        cache.insert(&first, handle("1"), start);
        cache.insert(&second, handle("2"), start + Duration::from_secs(1));
        // Touch `first` so `second` becomes the least recently used.
        assert!(
            cache
                .get::<SecretToken>(&first, start + Duration::from_secs(2))
                .is_some()
        );

        cache.insert(&third, handle("3"), start + Duration::from_secs(3));
        assert_eq!(cache.len(), 2);
        let later = start + Duration::from_secs(4);
        assert!(cache.get::<SecretToken>(&first, later).is_some());
        assert!(cache.get::<SecretToken>(&second, later).is_none());
        assert!(cache.get::<SecretToken>(&third, later).is_some());
    }

    #[test]
    fn replacing_an_existing_key_at_capacity_evicts_nothing() {
        let cache = HandleCache::new(TTL, 1);
        let now = Instant::now();
        let sel = selector();
        cache.insert(&sel, handle("1"), now);
        cache.insert(&sel, handle("2"), now);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn evict_selector_drops_only_that_selector() {
        let cache = HandleCache::new(TTL, 8);
        let now = Instant::now();
        let gone = selector();
        let kept = selector();
        cache.insert(&gone, handle("gone"), now);
        cache.insert(&kept, handle("kept"), now);

        cache.evict_selector(&gone);
        assert!(cache.get::<SecretToken>(&gone, now).is_none());
        assert!(cache.get::<SecretToken>(&kept, now).is_some());
    }
}
