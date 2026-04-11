//! Per-key registry of [`RecoveryGate`] instances.
//!
//! [`RecoveryGroupRegistry`] lazily creates a [`RecoveryGate`] for each
//! unique key, allowing callers to coordinate recovery per backend or
//! per resource without global synchronization.

use std::sync::Arc;

use dashmap::DashMap;

use super::gate::{RecoveryGate, RecoveryGateConfig};

/// Opaque key identifying a recovery group.
///
/// Typically derived from a resource key or backend address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecoveryGroupKey(String);

impl RecoveryGroupKey {
    /// Creates a new recovery group key.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Returns the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RecoveryGroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Registry of [`RecoveryGate`] instances keyed by [`RecoveryGroupKey`].
///
/// Thread-safe; uses `DashMap` for concurrent access.
///
/// # Examples
///
/// ```
/// use nebula_resource::recovery::{RecoveryGateConfig, RecoveryGroupKey, RecoveryGroupRegistry};
///
/// let registry = RecoveryGroupRegistry::new();
/// let gate = registry.get_or_create(
///     RecoveryGroupKey::new("postgres-primary"),
///     RecoveryGateConfig::default(),
/// );
/// ```
#[derive(Debug)]
pub struct RecoveryGroupRegistry {
    groups: DashMap<RecoveryGroupKey, Arc<RecoveryGate>>,
}

impl RecoveryGroupRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            groups: DashMap::new(),
        }
    }

    /// Returns the gate for `key`, creating one with `config` if absent.
    pub fn get_or_create(
        &self,
        key: RecoveryGroupKey,
        config: RecoveryGateConfig,
    ) -> Arc<RecoveryGate> {
        Arc::clone(
            self.groups
                .entry(key)
                .or_insert_with(|| Arc::new(RecoveryGate::new(config)))
                .value(),
        )
    }

    /// Returns the gate for `key` if it exists.
    pub fn get(&self, key: &RecoveryGroupKey) -> Option<Arc<RecoveryGate>> {
        self.groups.get(key).map(|r| Arc::clone(r.value()))
    }

    /// Removes the gate for `key`, returning it if it existed.
    pub fn remove(&self, key: &RecoveryGroupKey) -> Option<Arc<RecoveryGate>> {
        self.groups.remove(key).map(|(_, v)| v)
    }

    /// Returns the number of registered groups.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Returns `true` if no groups are registered.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

impl Default for RecoveryGroupRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_create_returns_same_gate() {
        let registry = RecoveryGroupRegistry::new();
        let key = RecoveryGroupKey::new("pg");

        let g1 = registry.get_or_create(key.clone(), RecoveryGateConfig::default());
        let g2 = registry.get_or_create(key, RecoveryGateConfig::default());

        // Same Arc allocation.
        assert!(Arc::ptr_eq(&g1, &g2));
    }

    #[test]
    fn get_returns_none_for_missing() {
        let registry = RecoveryGroupRegistry::new();
        assert!(registry.get(&RecoveryGroupKey::new("nope")).is_none());
    }

    #[test]
    fn remove_returns_gate_and_clears() {
        let registry = RecoveryGroupRegistry::new();
        let key = RecoveryGroupKey::new("redis");
        registry.get_or_create(key.clone(), RecoveryGateConfig::default());

        assert!(registry.remove(&key).is_some());
        assert!(registry.get(&key).is_none());
        assert!(registry.is_empty());
    }

    #[test]
    fn len_tracks_groups() {
        let registry = RecoveryGroupRegistry::new();
        assert_eq!(registry.len(), 0);

        registry.get_or_create(RecoveryGroupKey::new("a"), RecoveryGateConfig::default());
        registry.get_or_create(RecoveryGroupKey::new("b"), RecoveryGateConfig::default());
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn display_key() {
        let key = RecoveryGroupKey::new("my-backend");
        assert_eq!(key.to_string(), "my-backend");
        assert_eq!(key.as_str(), "my-backend");
    }
}
