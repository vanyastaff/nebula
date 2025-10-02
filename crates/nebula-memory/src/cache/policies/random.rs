//! Random cache eviction policy
//!
//! This module provides a simple random eviction policy that selects victims
//! uniformly at random. While not optimal for most caching scenarios, it provides
//! a baseline and can be useful for specific workloads where access patterns are
//! unpredictable or for testing purposes.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{collections::HashSet, hash::Hash, marker::PhantomData};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, vec::Vec},
    core::{hash::Hash, marker::PhantomData},
    hashbrown::HashSet,
};

use rand::Rng;

use crate::cache::compute::{CacheEntry, CacheKey};

use super::{EvictionEntry, EvictionPolicy, VictimSelector};

/// Configuration for Random eviction policy
#[derive(Debug, Clone)]
pub struct RandomConfig {
    /// Enable seeded random for reproducibility in tests
    pub seeded: bool,
    /// Seed value if seeded mode is enabled
    pub seed: u64,
}

impl Default for RandomConfig {
    fn default() -> Self {
        Self {
            seeded: false,
            seed: 0,
        }
    }
}

/// Random cache eviction policy
///
/// Selects victims uniformly at random. This is useful when:
/// - Access patterns are completely random
/// - You want a baseline for comparison
/// - Testing cache behavior
/// - Minimal memory overhead is required
///
/// # Examples
///
/// ```ignore
/// use nebula_memory::cache::policies::RandomPolicy;
///
/// let policy = RandomPolicy::<String, i32>::new();
/// // Use with cache...
/// ```
pub struct RandomPolicy<K, V>
where
    K: CacheKey,
{
    /// Phantom data for unused type parameter V
    _phantom: PhantomData<V>,
    /// Configuration
    config: RandomConfig,
    /// Set of tracked keys
    keys: HashSet<K>,
}

impl<K, V> RandomPolicy<K, V>
where
    K: CacheKey,
{
    /// Create a new random eviction policy
    pub fn new() -> Self {
        Self::with_config(RandomConfig::default())
    }

    /// Create a new random policy with custom configuration
    pub fn with_config(config: RandomConfig) -> Self {
        Self {
            _phantom: PhantomData,
            config,
            keys: HashSet::new(),
        }
    }

    /// Create a seeded random policy for reproducible behavior
    pub fn with_seed(seed: u64) -> Self {
        Self::with_config(RandomConfig { seeded: true, seed })
    }

    /// Get the number of tracked keys
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Clear all tracked keys
    pub fn clear(&mut self) {
        self.keys.clear();
    }
}

impl<K, V> Default for RandomPolicy<K, V>
where
    K: CacheKey,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> EvictionPolicy<K, V> for RandomPolicy<K, V>
where
    K: CacheKey + Send + Sync,
    V: Send + Sync,
{
    fn record_access(&mut self, _key: &K) {
        // Random policy doesn't care about access patterns
    }

    fn record_insertion(&mut self, key: &K, _entry: &CacheEntry<V>) {
        self.keys.insert(key.clone());
    }

    fn record_removal(&mut self, key: &K) {
        self.keys.remove(key);
    }

    fn as_victim_selector(&self) -> &dyn VictimSelector<K, V> {
        self
    }

    fn name(&self) -> &str {
        "Random"
    }
}

impl<K, V> VictimSelector<K, V> for RandomPolicy<K, V>
where
    K: CacheKey + Send + Sync,
    V: Send + Sync,
{
    fn select_victim<'a>(&self, entries: &[EvictionEntry<'a, K, V>]) -> Option<K> {
        if entries.is_empty() {
            return None;
        }

        let mut rng = rand::thread_rng();
        let index = rng.gen_range(0..entries.len());
        Some(entries[index].0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_policy_basic() {
        let mut policy = RandomPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        assert_eq!(policy.key_count(), 0);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        assert_eq!(policy.key_count(), 3);

        policy.record_removal(&"key2".to_string());
        assert_eq!(policy.key_count(), 2);
    }

    #[test]
    fn test_random_selection() {
        let policy = RandomPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        let entries: Vec<EvictionEntry<String, i32>> = vec![
            (&"key1".to_string(), &entry),
            (&"key2".to_string(), &entry),
            (&"key3".to_string(), &entry),
        ];

        // Should select one of the keys
        let victim = policy.as_victim_selector().select_victim(&entries);
        assert!(victim.is_some());

        let victim_key = victim.unwrap();
        assert!(victim_key == "key1" || victim_key == "key2" || victim_key == "key3");
    }

    #[test]
    fn test_empty_selection() {
        let policy = RandomPolicy::<String, i32>::new();
        let entries: Vec<EvictionEntry<String, i32>> = vec![];

        let victim = policy.as_victim_selector().select_victim(&entries);
        assert!(victim.is_none());
    }

    #[test]
    fn test_single_entry() {
        let policy = RandomPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        let entries: Vec<EvictionEntry<String, i32>> = vec![(&"only_key".to_string(), &entry)];

        let victim = policy.as_victim_selector().select_victim(&entries);
        assert_eq!(victim, Some("only_key".to_string()));
    }

    #[test]
    fn test_clear() {
        let mut policy = RandomPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        assert_eq!(policy.key_count(), 2);

        policy.clear();
        assert_eq!(policy.key_count(), 0);
    }

    #[test]
    fn test_name() {
        let policy = RandomPolicy::<String, i32>::new();
        assert_eq!(policy.name(), "Random");
    }
}
