//! Type-erased registry for managed resources.
//!
//! [`Registry`] stores managed resources indexed by [`ResourceKey`] and
//! [`TypeId`], supporting scope-aware lookup and typed downcasting.

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use dashmap::DashMap;
use nebula_core::ResourceKey;

use crate::{ctx::ScopeLevel, resource::Resource, runtime::managed::ManagedResource};

/// Type-erased trait for managed resources stored in the [`Registry`].
///
/// Every `ManagedResource<R>` implements this trait, allowing the registry
/// to store heterogeneous resource types behind a single `dyn AnyManagedResource`.
pub trait AnyManagedResource: Send + Sync + 'static {
    /// Returns the resource key for this managed resource.
    fn resource_key(&self) -> ResourceKey;

    /// Returns a reference to `self` as `&dyn Any` for downcasting.
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Returns the concrete `TypeId` used as the secondary index key.
    ///
    /// For real `ManagedResource<R>` this is `TypeId::of::<ManagedResource<R>>()`.
    /// Used by [`Registry::register`] to scrub stale rows from `type_index`
    /// when an entry is replaced in place (#382).
    fn managed_type_id(&self) -> TypeId;

    /// Type-erased lifecycle phase mutation (#387).
    ///
    /// Lets the manager drive phase transitions on all registered
    /// resources without needing a typed handle, which matters during
    /// graceful shutdown where only the type-erased registry iteration
    /// is available.
    fn set_phase_erased(&self, phase: crate::state::ResourcePhase);
}

impl<R: Resource> AnyManagedResource for ManagedResource<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn managed_type_id(&self) -> TypeId {
        TypeId::of::<ManagedResource<R>>()
    }

    fn set_phase_erased(&self, phase: crate::state::ResourcePhase) {
        self.set_phase(phase);
    }
}

/// A single entry in the registry, associating a scope with a managed resource.
struct RegistryEntry {
    scope: ScopeLevel,
    managed: Arc<dyn AnyManagedResource>,
}

/// Type-erased storage for all registered resources.
///
/// Provides two lookup paths:
/// - **By key + scope**: `get()` finds the best-matching entry for a given [`ResourceKey`] and
///   [`ScopeLevel`].
/// - **By type**: `get_typed()` uses a secondary [`TypeId`] index for typed lookup with automatic
///   downcasting.
pub struct Registry {
    /// Primary index: ResourceKey -> list of entries (one per scope).
    entries: DashMap<ResourceKey, Vec<RegistryEntry>>,
    /// Secondary index: TypeId -> ResourceKey (for typed lookup).
    type_index: DashMap<TypeId, ResourceKey>,
}

impl Registry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            type_index: DashMap::new(),
        }
    }

    /// Registers a managed resource under the given key, type, and scope.
    ///
    /// If an entry with the same key and scope already exists, it is replaced.
    pub fn register(
        &self,
        key: ResourceKey,
        type_id: TypeId,
        scope: ScopeLevel,
        managed: Arc<dyn AnyManagedResource>,
    ) {
        // Lock order is **strictly one-way**: `entries → (release) → type_index`.
        //
        // `get_typed` takes the `type_index` shard read lock first and only
        // then touches `entries`. If `register` ever held both dashmap
        // shards simultaneously in the opposite order, two concurrent
        // callers (one here, one in `get_typed`) could each be waiting on
        // the shard the other already holds — a classic lock-ordering
        // reversal. We prevent that by doing all `entries` work in a
        // scoped block, dropping the guard, and only *then* touching
        // `type_index`.
        //
        // #382 nuance: it's not enough to compare the replaced entry's
        // prior `TypeId` to the new one. If *another* scope under the
        // same key still holds a `ManagedResource<OldR>` instance, we
        // must NOT remove `OldR -> key` from `type_index` — doing so
        // would break `get_typed::<OldR>` for that other scope. So we
        // scan the rest of the entries while still holding the guard
        // and only mark the stale row for removal if nobody else uses
        // it.
        let stale_type_id = {
            let mut entries = self.entries.entry(key.clone()).or_default();

            if let Some(pos) = entries.iter().position(|e| e.scope == scope) {
                let prev_type_id = entries[pos].managed.managed_type_id();
                entries[pos] = RegistryEntry { scope, managed };

                if prev_type_id != type_id
                    && !entries
                        .iter()
                        .any(|e| e.managed.managed_type_id() == prev_type_id)
                {
                    Some(prev_type_id)
                } else {
                    None
                }
            } else {
                entries.push(RegistryEntry { scope, managed });
                None
            }
            // entries guard drops here.
        };

        if let Some(stale) = stale_type_id {
            self.type_index.remove_if(&stale, |_, k| k == &key);
        }
        self.type_index.insert(type_id, key);
    }

    /// Looks up a managed resource by key and scope.
    ///
    /// Returns the entry whose scope matches `scope` exactly. If no exact
    /// match is found, falls back to [`ScopeLevel::Global`].
    pub fn get(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Option<Arc<dyn AnyManagedResource>> {
        let entries = self.entries.get(key)?;
        Self::find_by_scope(&entries, scope)
    }

    /// Typed lookup: finds the resource for type `R` and downcasts to
    /// `Arc<ManagedResource<R>>`.
    ///
    /// Uses the [`TypeId`] secondary index to find the key, then performs
    /// a scope-aware lookup and downcast.
    pub fn get_typed<R: Resource>(&self, scope: &ScopeLevel) -> Option<Arc<ManagedResource<R>>> {
        let type_id = TypeId::of::<ManagedResource<R>>();
        let key = self.type_index.get(&type_id)?;
        let any_managed = self.get(&key, scope)?;
        any_managed
            .as_any_arc()
            .downcast::<ManagedResource<R>>()
            .ok()
    }

    /// Removes all entries for the given key.
    ///
    /// Returns `true` if the key existed and was removed, `false` otherwise.
    /// Also removes the type index entry if it points to this key.
    pub fn remove(&self, key: &ResourceKey) -> bool {
        let existed = self.entries.remove(key).is_some();
        if existed {
            self.type_index.retain(|_type_id, k| k != key);
        }
        existed
    }

    /// Returns all registered resource keys.
    pub fn keys(&self) -> Vec<ResourceKey> {
        self.entries.iter().map(|r| r.key().clone()).collect()
    }

    /// Returns every registered managed resource across all scopes.
    ///
    /// Used by the manager to drive lifecycle transitions (e.g. shifting
    /// every resource to `Draining` / `ShuttingDown` during graceful
    /// shutdown, #387) without needing typed access to each entry.
    pub(crate) fn all_managed(&self) -> Vec<Arc<dyn AnyManagedResource>> {
        let mut out = Vec::new();
        for row in &self.entries {
            for entry in row.value() {
                out.push(Arc::clone(&entry.managed));
            }
        }
        out
    }

    /// Returns `true` if a resource with the given key is registered.
    pub fn contains(&self, key: &ResourceKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Removes all entries from the registry.
    ///
    /// This drops every `Arc<dyn AnyManagedResource>`, releasing their
    /// resources (including `Arc<ReleaseQueue>` references).
    pub fn clear(&self) {
        self.entries.clear();
        self.type_index.clear();
    }

    /// Scope-aware lookup within a list of entries.
    ///
    /// Prefers an exact scope match; falls back to [`ScopeLevel::Global`].
    fn find_by_scope(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
    ) -> Option<Arc<dyn AnyManagedResource>> {
        // Exact scope match first.
        if let Some(entry) = entries.iter().find(|e| e.scope == *scope) {
            return Some(Arc::clone(&entry.managed));
        }

        // Fallback to Global scope.
        if *scope != ScopeLevel::Global
            && let Some(entry) = entries.iter().find(|e| e.scope == ScopeLevel::Global)
        {
            return Some(Arc::clone(&entry.managed));
        }

        None
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeA;
    struct FakeB;

    impl AnyManagedResource for FakeA {
        fn resource_key(&self) -> ResourceKey {
            ResourceKey::new("fake").unwrap()
        }
        fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
        fn managed_type_id(&self) -> TypeId {
            TypeId::of::<FakeA>()
        }
        fn set_phase_erased(&self, _phase: crate::state::ResourcePhase) {}
    }

    impl AnyManagedResource for FakeB {
        fn resource_key(&self) -> ResourceKey {
            ResourceKey::new("fake").unwrap()
        }
        fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
        fn managed_type_id(&self) -> TypeId {
            TypeId::of::<FakeB>()
        }
        fn set_phase_erased(&self, _phase: crate::state::ResourcePhase) {}
    }

    #[test]
    fn register_replace_preserves_type_id_still_used_by_another_scope() {
        // Regression for a correctness hole raised in PR #399 review:
        // if scope A and scope B both hold `TypeA`, replacing scope A
        // with `TypeB` must NOT scrub `TypeA -> key` from `type_index`,
        // otherwise `get_typed::<TypeA>(B)` would break.
        let reg = Registry::new();
        let key = ResourceKey::new("fake").unwrap();

        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            ScopeLevel::Global,
            Arc::new(FakeA),
        );
        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            ScopeLevel::Workspace("p".into()),
            Arc::new(FakeA),
        );

        // Replace only the Global entry with FakeB. Workflow still
        // holds FakeA, so the TypeA row in type_index must survive.
        reg.register(
            key,
            TypeId::of::<FakeB>(),
            ScopeLevel::Global,
            Arc::new(FakeB),
        );

        assert!(
            reg.type_index.contains_key(&TypeId::of::<FakeA>()),
            "TypeA row must survive because the Workspace scope still uses it",
        );
        assert!(reg.type_index.contains_key(&TypeId::of::<FakeB>()));
    }

    #[test]
    fn register_replace_drops_stale_type_id_row() {
        let reg = Registry::new();
        let key = ResourceKey::new("fake").unwrap();
        let scope = ScopeLevel::Global;

        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            scope.clone(),
            Arc::new(FakeA),
        );
        assert!(reg.type_index.contains_key(&TypeId::of::<FakeA>()));

        // Replace at the same key+scope with a different concrete type.
        reg.register(key, TypeId::of::<FakeB>(), scope, Arc::new(FakeB));

        // The stale TypeId row for FakeA must be gone (#382).
        assert!(
            !reg.type_index.contains_key(&TypeId::of::<FakeA>()),
            "stale TypeId for FakeA still in type_index after replace"
        );
        assert!(reg.type_index.contains_key(&TypeId::of::<FakeB>()));
    }
}
