//! FIFO (First In First Out) cache eviction policy
//!
//! This module provides a simple FIFO eviction policy that evicts entries
//! in the order they were inserted. This is useful for scenarios where
//! temporal freshness matters more than access frequency.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::{HashMap, VecDeque},
    marker::PhantomData,
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, collections::VecDeque, vec::Vec},
    core::{hash::Hash, marker::PhantomData},
    hashbrown::HashMap,
};

use crate::cache::compute::{CacheEntry, CacheKey};

use super::{EvictionEntry, EvictionPolicy, VictimSelector};

/// Configuration for FIFO eviction policy
#[derive(Debug, Clone)]
pub struct FifoConfig {
    /// Maximum number of tracked items
    pub max_tracked_items: usize,
    /// Enable insertion timestamp tracking
    pub track_insertion_order: bool,
}

impl Default for FifoConfig {
    fn default() -> Self {
        Self {
            max_tracked_items: 10000,
            track_insertion_order: true,
        }
    }
}

/// FIFO (First In First Out) cache eviction policy
///
/// Evicts entries in insertion order. This is useful when:
/// - Temporal freshness is more important than access frequency
/// - Simple, predictable eviction behavior is desired
/// - Memory overhead must be minimal
///
/// # Examples
///
/// ```ignore
/// use nebula_memory::cache::policies::FifoPolicy;
///
/// let policy = FifoPolicy::<String, i32>::new();
/// // Use with cache...
/// ```
pub struct FifoPolicy<K, V>
where
    K: CacheKey,
{
    /// Phantom data for unused type parameter V
    _phantom: PhantomData<V>,
    /// Configuration
    config: FifoConfig,
    /// Queue of keys in insertion order
    queue: VecDeque<K>,
    /// Set of keys for fast lookup
    key_set: HashMap<K, u64>, // key -> insertion_order
    /// Insertion counter
    insertion_counter: u64,
}

impl<K, V> FifoPolicy<K, V>
where
    K: CacheKey,
{
    /// Create a new FIFO eviction policy
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(FifoConfig::default())
    }

    /// Create a new FIFO policy with custom configuration
    #[must_use]
    pub fn with_config(config: FifoConfig) -> Self {
        Self {
            _phantom: PhantomData,
            config,
            queue: VecDeque::new(),
            key_set: HashMap::new(),
            insertion_counter: 0,
        }
    }

    /// Create a FIFO policy with specific capacity
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_config(FifoConfig {
            max_tracked_items: capacity,
            track_insertion_order: true,
        })
    }

    /// Get the number of tracked keys
    #[must_use]
    pub fn key_count(&self) -> usize {
        self.queue.len()
    }

    /// Get the oldest key (next victim candidate)
    #[must_use]
    pub fn peek_oldest(&self) -> Option<&K> {
        self.queue.front()
    }

    /// Clear all tracked keys
    pub fn clear(&mut self) {
        self.queue.clear();
        self.key_set.clear();
        self.insertion_counter = 0;
    }

    /// Get insertion order of a key
    pub fn get_insertion_order(&self, key: &K) -> Option<u64> {
        self.key_set.get(key).copied()
    }
}

impl<K, V> Default for FifoPolicy<K, V>
where
    K: CacheKey,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> EvictionPolicy<K, V> for FifoPolicy<K, V>
where
    K: CacheKey + Send + Sync,
    V: Send + Sync,
{
    fn record_access(&mut self, _key: &K) {
        // FIFO doesn't care about access patterns
    }

    fn record_insertion(&mut self, key: &K, _entry: &CacheEntry<V>) {
        // Only track if not already present
        if !self.key_set.contains_key(key) {
            self.queue.push_back(key.clone());
            self.key_set.insert(key.clone(), self.insertion_counter);
            self.insertion_counter += 1;

            // Limit queue size if needed
            while self.queue.len() > self.config.max_tracked_items {
                if let Some(old_key) = self.queue.pop_front() {
                    self.key_set.remove(&old_key);
                }
            }
        }
    }

    fn record_removal(&mut self, key: &K) {
        if self.key_set.remove(key).is_some() {
            // Remove from queue (O(n) but typically small)
            if let Some(pos) = self.queue.iter().position(|k| k == key) {
                self.queue.remove(pos);
            }
        }
    }

    fn as_victim_selector(&self) -> &dyn VictimSelector<K, V> {
        self
    }

    fn name(&self) -> &'static str {
        "FIFO"
    }
}

impl<K, V> VictimSelector<K, V> for FifoPolicy<K, V>
where
    K: CacheKey + Send + Sync,
    V: Send + Sync,
{
    fn select_victim(&self, entries: &[EvictionEntry<'_, K, V>]) -> Option<K> {
        if entries.is_empty() {
            return None;
        }

        // Find the entry with the oldest insertion order
        entries
            .iter()
            .filter_map(|(key, _entry)| self.key_set.get(*key).map(|&order| (*key, order)))
            .min_by_key(|(_, order)| *order)
            .map(|(key, _)| key.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fifo_policy_basic() {
        let mut policy = FifoPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        assert_eq!(policy.key_count(), 0);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        assert_eq!(policy.key_count(), 3);

        // key1 is oldest
        assert_eq!(policy.peek_oldest(), Some(&"key1".to_string()));
    }

    #[test]
    fn test_fifo_eviction_order() {
        let policy = FifoPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        // Build entries in insertion order
        let key1 = "key1".to_string();
        let key2 = "key2".to_string();
        let key3 = "key3".to_string();
        let entries: Vec<EvictionEntry<String, i32>> =
            vec![(&key1, &entry), (&key2, &entry), (&key3, &entry)];

        // Manually track insertion order
        let mut policy_mut = FifoPolicy::<String, i32>::new();
        policy_mut.record_insertion(&"key1".to_string(), &entry);
        policy_mut.record_insertion(&"key2".to_string(), &entry);
        policy_mut.record_insertion(&"key3".to_string(), &entry);

        // Should select key1 (oldest)
        let victim = policy_mut.as_victim_selector().select_victim(&entries);
        assert_eq!(victim, Some("key1".to_string()));
    }

    #[test]
    fn test_fifo_with_removal() {
        let mut policy = FifoPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        // Remove the oldest
        policy.record_removal(&"key1".to_string());

        assert_eq!(policy.key_count(), 2);
        assert_eq!(policy.peek_oldest(), Some(&"key2".to_string()));
    }

    #[test]
    fn test_fifo_access_doesnt_affect_order() {
        let mut policy = FifoPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);

        // Access key1 multiple times - shouldn't change order
        policy.record_access(&"key1".to_string());
        policy.record_access(&"key1".to_string());
        policy.record_access(&"key1".to_string());

        // key1 should still be oldest
        assert_eq!(policy.peek_oldest(), Some(&"key1".to_string()));
    }

    #[test]
    fn test_insertion_order_tracking() {
        let mut policy = FifoPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        let order1 = policy.get_insertion_order(&"key1".to_string()).unwrap();
        let order2 = policy.get_insertion_order(&"key2".to_string()).unwrap();
        let order3 = policy.get_insertion_order(&"key3".to_string()).unwrap();

        assert!(order1 < order2);
        assert!(order2 < order3);
    }

    #[test]
    fn test_capacity_limit() {
        let mut policy = FifoPolicy::<String, i32>::with_capacity(3);
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);
        policy.record_insertion(&"key4".to_string(), &entry); // Should evict key1

        assert_eq!(policy.key_count(), 3);
        assert_eq!(policy.peek_oldest(), Some(&"key2".to_string()));
    }

    #[test]
    fn test_clear() {
        let mut policy = FifoPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        assert_eq!(policy.key_count(), 2);

        policy.clear();
        assert_eq!(policy.key_count(), 0);
        assert!(policy.peek_oldest().is_none());
    }

    #[test]
    fn test_name() {
        let policy = FifoPolicy::<String, i32>::new();
        assert_eq!(policy.name(), "FIFO");
    }

    #[test]
    fn test_empty_selection() {
        let policy = FifoPolicy::<String, i32>::new();
        let entries: Vec<EvictionEntry<String, i32>> = vec![];

        let victim = policy.as_victim_selector().select_victim(&entries);
        assert!(victim.is_none());
    }
}
