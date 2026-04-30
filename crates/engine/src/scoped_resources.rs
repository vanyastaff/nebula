//! Scoped resource resolution surface (M6.1 — Phase 6 wiring).
//!
//! Phase 6 lands the **resolution precedence** layer (`scoped → global`)
//! used by [`ActionContextExt::acquire_resource_by_id`](nebula_action::ActionContextExt::acquire_resource_by_id)
//! and [`HasResourcesExt::resource`](nebula_resource::HasResourcesExt::resource).
//! The actual `DashMap`-backed scoped storage and per-branch lifecycle
//! (push / pop / cleanup ordering) is Phase 7 (M6.2, plan tasks 7.1-7.5).
//!
//! ## Architecture
//!
//! - [`ScopedResourceMap`] — dyn-safe trait that walks the scope chain (closest ancestor first)
//!   looking for a registered resource. Phase 6 ships [`EmptyScopedResourceMap`] (always misses);
//!   Phase 7 swaps in the real per-branch implementation.
//! - [`LayeredResourceAccessor`] — wraps `(Arc<dyn ScopedResourceMap>, Arc<dyn ResourceAccessor>)`
//!   and implements [`ResourceAccessor`]. On lookup, consults the scoped map first; on miss,
//!   delegates to the global accessor (typically
//!   [`EngineResourceAccessor`](crate::EngineResourceAccessor)).
//!
//! ## Precedence rule
//!
//! `scoped → global`, **closest ancestor wins**. The scope chain is
//! `Execution > Workflow > Workspace > Organization > Global`. A resource
//! registered at `Execution` level shadows the same key at `Workflow`
//! level, which shadows `Workspace`, etc. If no scope owns the key, the
//! global accessor is consulted.
//!
//! ## Why a trait, not a concrete type
//!
//! Phase 7 needs concurrency-safe per-branch storage (`DashMap` keyed by
//! `(NodeKey, ResourceKey)`). Decoupling the lookup interface here lets
//! Phase 6 wire the precedence logic against an empty stub today and
//! Phase 7 plug in the real storage without re-touching action / engine
//! call sites.

use std::{any::Any, fmt, future::Future, pin::Pin, sync::Arc};

use nebula_core::{CoreError, ResourceKey, accessor::ResourceAccessor};

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type-erased lookup payload returned by [`ScopedResourceMap::lookup_in_ancestors`].
///
/// Mirrors the shape returned by [`ResourceAccessor::acquire_any`] so the
/// layered accessor can pass results through to the caller without a
/// second wrapping layer.
pub type ScopedLookup = Box<dyn Any + Send + Sync>;

/// Closest-ancestor lookup over a workflow's scope chain.
///
/// Implementations walk the scope chain (current node → parent branch →
/// workflow → … → global) and return the first registered resource
/// matching `key`. Phase 7 ships [`crate::scoped_resources`] with the
/// concrete `DashMap`-based per-branch implementation; Phase 6 ships
/// [`EmptyScopedResourceMap`] as the no-op default.
///
/// # Contract
///
/// - `lookup_in_ancestors` returns `Ok(Some(_))` when the key is registered at any ancestor scope.
/// - `Ok(None)` means *no scope owns the key* — the caller should fall through to the global
///   accessor.
/// - `Err(_)` is reserved for genuine lookup faults (e.g., poisoned scope storage); do not use
///   `Err` to signal "missing".
///
/// `has_in_ancestors` mirrors the lookup but returns a plain `bool` for
/// existence checks that do not need the payload.
pub trait ScopedResourceMap: Send + Sync + fmt::Debug {
    /// Walk ancestor scopes for `key`; closest-ancestor wins.
    ///
    /// Returns `Ok(Some(payload))` on hit, `Ok(None)` on miss, `Err(_)`
    /// on lookup fault.
    fn lookup_in_ancestors<'a>(
        &'a self,
        key: &'a ResourceKey,
    ) -> BoxFut<'a, Result<Option<ScopedLookup>, CoreError>>;

    /// Existence check across ancestor scopes.
    fn has_in_ancestors(&self, key: &ResourceKey) -> bool;
}

/// No-op [`ScopedResourceMap`] — every lookup misses.
///
/// Phase 6 default. The layered accessor wired with this stub behaves
/// identically to a global-only accessor; Phase 7 replaces it with the
/// real per-branch storage and lifecycle.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyScopedResourceMap;

impl ScopedResourceMap for EmptyScopedResourceMap {
    fn lookup_in_ancestors<'a>(
        &'a self,
        _key: &'a ResourceKey,
    ) -> BoxFut<'a, Result<Option<ScopedLookup>, CoreError>> {
        Box::pin(async { Ok(None) })
    }

    fn has_in_ancestors(&self, _key: &ResourceKey) -> bool {
        false
    }
}

/// `ResourceAccessor` impl that consults a [`ScopedResourceMap`] before
/// falling through to a global accessor.
///
/// # Lookup order
///
/// 1. `scoped.lookup_in_ancestors(key)` — closest-ancestor walk.
/// 2. On miss (`Ok(None)`), `global.acquire_any(key)`.
///
/// This is the accessor injected into [`ActionRuntimeContext`](nebula_action::ActionRuntimeContext)
/// from Phase 6 onwards. Action authors do not see the layering — they
/// call `ctx.resource::<R>()` / `ctx.acquire_resource_by_id::<R>(id)` and
/// the precedence is applied transparently.
///
/// # Examples
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use nebula_engine::{
///     EngineResourceAccessor, scoped_resources::{EmptyScopedResourceMap, LayeredResourceAccessor},
/// };
/// use nebula_resource::Manager;
///
/// let manager = Arc::new(Manager::new());
/// let global = Arc::new(EngineResourceAccessor::new(manager));
/// let scoped = Arc::new(EmptyScopedResourceMap);
/// let layered = Arc::new(LayeredResourceAccessor::new(scoped, global));
/// // Inject into ActionRuntimeContext::with_resources(layered)
/// ```
pub struct LayeredResourceAccessor {
    scoped: Arc<dyn ScopedResourceMap>,
    global: Arc<dyn ResourceAccessor>,
}

impl LayeredResourceAccessor {
    /// Build a layered accessor from the scoped map and global fallthrough.
    #[must_use]
    pub fn new(scoped: Arc<dyn ScopedResourceMap>, global: Arc<dyn ResourceAccessor>) -> Self {
        Self { scoped, global }
    }

    /// Convenience for the Phase 6 default (no per-scope storage yet).
    ///
    /// Equivalent to `LayeredResourceAccessor::new(Arc::new(EmptyScopedResourceMap), global)`.
    #[must_use]
    pub fn global_only(global: Arc<dyn ResourceAccessor>) -> Self {
        Self::new(Arc::new(EmptyScopedResourceMap), global)
    }
}

impl fmt::Debug for LayeredResourceAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayeredResourceAccessor")
            .field("scoped", &self.scoped)
            .field("global", &"<dyn ResourceAccessor>")
            .finish()
    }
}

impl ResourceAccessor for LayeredResourceAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        self.scoped.has_in_ancestors(key) || self.global.has(key)
    }

    fn acquire_any(&self, key: &ResourceKey) -> BoxFut<'_, Result<ScopedLookup, CoreError>> {
        let key_owned = key.clone();
        Box::pin(async move {
            match self.scoped.lookup_in_ancestors(&key_owned).await? {
                Some(payload) => Ok(payload),
                None => self.global.acquire_any(&key_owned).await,
            }
        })
    }

    fn try_acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<ScopedLookup>, CoreError>> {
        let key_owned = key.clone();
        Box::pin(async move {
            match self.scoped.lookup_in_ancestors(&key_owned).await? {
                Some(payload) => Ok(Some(payload)),
                None => self.global.try_acquire_any(&key_owned).await,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    /// Test fixture: scoped map that holds a single registered key with a
    /// boxed marker payload.
    #[derive(Debug)]
    struct OneKeyScopedMap {
        registered: ResourceKey,
        payload_marker: u64,
        hits: AtomicUsize,
    }

    impl OneKeyScopedMap {
        fn new(registered: ResourceKey, payload_marker: u64) -> Self {
            Self {
                registered,
                payload_marker,
                hits: AtomicUsize::new(0),
            }
        }
    }

    impl ScopedResourceMap for OneKeyScopedMap {
        fn lookup_in_ancestors<'a>(
            &'a self,
            key: &'a ResourceKey,
        ) -> BoxFut<'a, Result<Option<ScopedLookup>, CoreError>> {
            Box::pin(async move {
                if key == &self.registered {
                    self.hits.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(Box::new(self.payload_marker) as ScopedLookup))
                } else {
                    Ok(None)
                }
            })
        }

        fn has_in_ancestors(&self, key: &ResourceKey) -> bool {
            key == &self.registered
        }
    }

    /// Test fixture: global accessor that stores keyed `u64` markers.
    struct TestGlobalAccessor {
        registered: ResourceKey,
        payload_marker: u64,
        hits: AtomicUsize,
    }

    impl TestGlobalAccessor {
        fn new(registered: ResourceKey, payload_marker: u64) -> Self {
            Self {
                registered,
                payload_marker,
                hits: AtomicUsize::new(0),
            }
        }
    }

    impl fmt::Debug for TestGlobalAccessor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("TestGlobalAccessor")
                .field("registered", &self.registered)
                .finish()
        }
    }

    impl ResourceAccessor for TestGlobalAccessor {
        fn has(&self, key: &ResourceKey) -> bool {
            key == &self.registered
        }

        fn acquire_any(&self, key: &ResourceKey) -> BoxFut<'_, Result<ScopedLookup, CoreError>> {
            let key_owned = key.clone();
            Box::pin(async move {
                if key_owned == self.registered {
                    self.hits.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(self.payload_marker) as ScopedLookup)
                } else {
                    Err(CoreError::CredentialNotFound {
                        key: key_owned.as_str().to_owned(),
                    })
                }
            })
        }

        fn try_acquire_any(
            &self,
            key: &ResourceKey,
        ) -> BoxFut<'_, Result<Option<ScopedLookup>, CoreError>> {
            let key_owned = key.clone();
            Box::pin(async move {
                if key_owned == self.registered {
                    Ok(Some(Box::new(self.payload_marker) as ScopedLookup))
                } else {
                    Ok(None)
                }
            })
        }
    }

    fn rk(key: &str) -> ResourceKey {
        ResourceKey::new(key).expect("valid resource key in test")
    }

    fn marker(boxed: ScopedLookup) -> u64 {
        *boxed
            .downcast::<u64>()
            .expect("test fixture stores u64 markers")
    }

    #[tokio::test]
    async fn empty_scoped_map_always_misses() {
        let map = EmptyScopedResourceMap;
        let key = rk("postgres");
        assert!(!map.has_in_ancestors(&key));
        assert!(map.lookup_in_ancestors(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn scoped_only_hit_returns_scoped_payload() {
        let key = rk("postgres");
        let scoped = Arc::new(OneKeyScopedMap::new(key.clone(), 0xaaaa));
        let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
        let layered = LayeredResourceAccessor::new(scoped.clone(), global.clone());

        let payload = layered.acquire_any(&key).await.unwrap();
        assert_eq!(marker(payload), 0xaaaa);
        assert_eq!(scoped.hits.load(Ordering::SeqCst), 1);
        assert_eq!(global.hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn global_only_hit_falls_through() {
        let scoped_key = rk("postgres");
        let global_key = rk("redis");
        let scoped = Arc::new(OneKeyScopedMap::new(scoped_key, 0xaaaa));
        let global = Arc::new(TestGlobalAccessor::new(global_key.clone(), 0xbbbb));
        let layered = LayeredResourceAccessor::new(scoped.clone(), global.clone());

        let payload = layered.acquire_any(&global_key).await.unwrap();
        assert_eq!(marker(payload), 0xbbbb);
        assert_eq!(global.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scoped_wins_over_global_at_same_key() {
        let key = rk("postgres");
        // Both sides claim "postgres" with different payload markers.
        let scoped = Arc::new(OneKeyScopedMap::new(key.clone(), 0xaaaa));
        let global = Arc::new(TestGlobalAccessor::new(key.clone(), 0xbbbb));
        let layered = LayeredResourceAccessor::new(scoped.clone(), global.clone());

        let payload = layered.acquire_any(&key).await.unwrap();
        assert_eq!(marker(payload), 0xaaaa, "scoped layer must win");
        // Global must NOT have been consulted.
        assert_eq!(global.hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn missing_in_both_returns_error() {
        let scoped = Arc::new(OneKeyScopedMap::new(rk("postgres"), 0xaaaa));
        let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
        let layered = LayeredResourceAccessor::new(scoped, global);

        let result = layered.acquire_any(&rk("kafka")).await;
        assert!(
            matches!(result, Err(CoreError::CredentialNotFound { .. })),
            "expected CredentialNotFound, got {result:?}"
        );
    }

    #[tokio::test]
    async fn try_acquire_any_returns_none_when_missing_in_both() {
        let scoped = Arc::new(OneKeyScopedMap::new(rk("postgres"), 0xaaaa));
        let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
        let layered = LayeredResourceAccessor::new(scoped, global);

        let result = layered.try_acquire_any(&rk("kafka")).await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn has_walks_both_layers() {
        let scoped = Arc::new(OneKeyScopedMap::new(rk("postgres"), 0xaaaa));
        let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
        let layered = LayeredResourceAccessor::new(scoped, global);

        assert!(layered.has(&rk("postgres")), "scoped layer reports key");
        assert!(layered.has(&rk("redis")), "global layer reports key");
        assert!(!layered.has(&rk("kafka")), "neither layer has key");
    }

    #[test]
    fn global_only_constructor_uses_empty_scoped() {
        let global = Arc::new(TestGlobalAccessor::new(rk("redis"), 0xbbbb));
        let layered = LayeredResourceAccessor::global_only(global);
        // Equivalent to the explicit two-arg form with EmptyScopedResourceMap.
        assert!(!layered.has(&rk("postgres"))); // not in global, not in (empty) scope
        assert!(layered.has(&rk("redis"))); // global hit
    }
}
