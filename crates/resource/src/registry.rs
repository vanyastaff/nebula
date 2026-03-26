//! Type-erased registry for managed resources.
//!
//! [`Registry`] stores managed resources indexed by [`ResourceKey`] and
//! [`TypeId`], supporting scope-aware lookup and typed downcasting.

use std::any::{Any, TypeId};
use std::sync::Arc;

use dashmap::DashMap;
use nebula_core::ResourceKey;

use crate::ctx::ScopeLevel;
use crate::metrics::ResourceMetrics;
use crate::resource::Resource;
use crate::runtime::managed::ManagedResource;

/// Type-erased trait for managed resources stored in the [`Registry`].
///
/// Every `ManagedResource<R>` implements this trait, allowing the registry
/// to store heterogeneous resource types behind a single `dyn AnyManagedResource`.
pub trait AnyManagedResource: Send + Sync + 'static {
    /// Returns the resource key for this managed resource.
    fn resource_key(&self) -> ResourceKey;

    /// Returns a reference to `self` as `&dyn Any` for downcasting.
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Returns the per-resource metrics for this managed resource.
    fn metrics(&self) -> &Arc<ResourceMetrics>;
}

impl<R: Resource> AnyManagedResource for ManagedResource<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn metrics(&self) -> &Arc<ResourceMetrics> {
        &self.metrics
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
/// - **By key + scope**: `get()` finds the best-matching entry for a given
///   [`ResourceKey`] and [`ScopeLevel`].
/// - **By type**: `get_typed()` uses a secondary [`TypeId`] index for
///   typed lookup with automatic downcasting.
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
        self.type_index.insert(type_id, key.clone());

        let mut entries = self.entries.entry(key).or_default();
        // Replace existing entry with same scope, if any.
        if let Some(pos) = entries.iter().position(|e| e.scope == scope) {
            entries[pos] = RegistryEntry { scope, managed };
        } else {
            entries.push(RegistryEntry { scope, managed });
        }
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
