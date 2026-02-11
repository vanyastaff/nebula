//! Simple LRU (Least Recently Used) cache eviction policy
//!
//! This module provides a straightforward LRU implementation using a `VecDeque`.
//! When the cache is full, it evicts the least recently used entry.
//!
//! This is a simplified version that focuses on clarity and correctness over
//! advanced optimizations. For most use cases, this simple approach is sufficient.

use std::{
    collections::{HashMap, VecDeque},
    marker::PhantomData,
};

use crate::cache::compute::{CacheEntry, CacheKey};

use super::{EvictionEntry, EvictionPolicy, VictimSelector};

/// Simple LRU (Least Recently Used) cache eviction policy
///
/// Maintains a queue of keys ordered by recency of access. When an entry is accessed,
/// it's moved to the front of the queue. When eviction is needed, the key at the back
/// (least recently used) is selected.
///
/// This implementation uses a `VecDeque` for simplicity and clarity. For extremely
/// high-throughput scenarios with millions of accesses per second, a doubly-linked
/// list might be more efficient, but for typical caching use cases this is sufficient.
///
/// # Examples
///
/// ```ignore
/// use nebula_memory::cache::policies::LruPolicy;
///
/// let mut policy = LruPolicy::<String, i32>::new();
///
/// // Record accesses
/// policy.record_access(&"key1".to_string());
/// policy.record_access(&"key2".to_string());
/// policy.record_access(&"key1".to_string()); // key1 is now most recent
///
/// // key2 would be evicted first (least recently used)
/// ```
pub struct LruPolicy<K, V>
where
    K: CacheKey,
{
    /// Phantom data for unused type parameter V
    _phantom: PhantomData<V>,
    /// Queue of keys in LRU order (front = most recent, back = least recent)
    queue: VecDeque<K>,
    /// Set of keys for O(1) membership testing
    key_set: HashMap<K, ()>,
}

impl<K, V> LruPolicy<K, V>
where
    K: CacheKey,
{
    /// Create a new LRU eviction policy
    #[must_use]
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
            queue: VecDeque::new(),
            key_set: HashMap::new(),
        }
    }

    /// Get the number of tracked keys
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Check if the policy is tracking any keys
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Clear all tracked keys
    pub fn clear(&mut self) {
        self.queue.clear();
        self.key_set.clear();
    }

    /// Move a key to the front (mark as most recently used)
    fn touch(&mut self, key: &K) {
        // Remove key from its current position
        self.queue.retain(|k| k != key);
        // Add it to the front (most recent)
        self.queue.push_front(key.clone());
    }
}

impl<K, V> Default for LruPolicy<K, V>
where
    K: CacheKey,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> EvictionPolicy<K, V> for LruPolicy<K, V>
where
    K: CacheKey + Send + Sync,
    V: Send + Sync,
{
    fn record_access(&mut self, key: &K) {
        // Only touch if key exists
        if self.key_set.contains_key(key) {
            self.touch(key);
        }
    }

    fn record_insertion(&mut self, key: &K, _entry: &CacheEntry<V>) {
        // Add to front (most recently used)
        self.queue.push_front(key.clone());
        self.key_set.insert(key.clone(), ());
    }

    fn record_removal(&mut self, key: &K) {
        self.queue.retain(|k| k != key);
        self.key_set.remove(key);
    }

    fn as_victim_selector(&self) -> &dyn VictimSelector<K, V> {
        self
    }

    fn name(&self) -> &'static str {
        "LRU"
    }
}

impl<K, V> VictimSelector<K, V> for LruPolicy<K, V>
where
    K: CacheKey + Send + Sync,
    V: Send + Sync,
{
    fn select_victim(&self, _entries: &[EvictionEntry<'_, K, V>]) -> Option<K> {
        // Return the least recently used key (back of queue)
        self.queue.back().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_basic() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        assert_eq!(policy.len(), 0);
        assert!(policy.is_empty());

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        assert_eq!(policy.len(), 3);
        assert!(!policy.is_empty());

        // Most recent order: key3, key2, key1
        // Least recent (victim) should be key1
        let victim = policy.as_victim_selector().select_victim(&[]);
        assert_eq!(victim, Some("key1".to_string()));
    }

    #[test]
    fn test_lru_access_updates_order() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        // Access key1 - it should become most recent
        policy.record_access(&"key1".to_string());

        // Now order is: key1, key3, key2
        // Least recent (victim) should be key2
        let victim = policy.as_victim_selector().select_victim(&[]);
        assert_eq!(victim, Some("key2".to_string()));
    }

    #[test]
    fn test_lru_removal() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        assert_eq!(policy.len(), 3);

        policy.record_removal(&"key2".to_string());
        assert_eq!(policy.len(), 2);

        // key2 should not be returned as victim
        let victim = policy.as_victim_selector().select_victim(&[]);
        assert!(victim == Some("key1".to_string()) || victim == Some("key3".to_string()));
        assert_ne!(victim, Some("key2".to_string()));
    }

    #[test]
    fn test_lru_clear() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        assert_eq!(policy.len(), 2);

        policy.clear();
        assert_eq!(policy.len(), 0);
        assert!(policy.is_empty());

        let victim = policy.as_victim_selector().select_victim(&[]);
        assert_eq!(victim, None);
    }

    #[test]
    fn test_lru_empty_selection() {
        let policy = LruPolicy::<String, i32>::new();
        let entries: Vec<EvictionEntry<String, i32>> = vec![];

        let victim = policy.as_victim_selector().select_victim(&entries);
        assert!(victim.is_none());
    }

    #[test]
    fn test_lru_single_entry() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"only_key".to_string(), &entry);

        let victim = policy.as_victim_selector().select_victim(&[]);
        assert_eq!(victim, Some("only_key".to_string()));
    }

    #[test]
    fn test_lru_multiple_accesses() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);
        policy.record_insertion(&"key3".to_string(), &entry);

        // Access key1 multiple times - it should stay most recent
        policy.record_access(&"key1".to_string());
        policy.record_access(&"key1".to_string());
        policy.record_access(&"key1".to_string());

        // key2 should still be least recent
        let victim = policy.as_victim_selector().select_victim(&[]);
        assert_eq!(victim, Some("key2".to_string()));
    }

    #[test]
    fn test_lru_interleaved_access() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"a".to_string(), &entry);
        policy.record_insertion(&"b".to_string(), &entry);
        policy.record_insertion(&"c".to_string(), &entry);

        // Order: c, b, a

        policy.record_access(&"a".to_string()); // Order: a, c, b
        policy.record_access(&"c".to_string()); // Order: c, a, b
        policy.record_access(&"b".to_string()); // Order: b, c, a

        // Now 'a' is least recent
        let victim = policy.as_victim_selector().select_victim(&[]);
        assert_eq!(victim, Some("a".to_string()));
    }

    #[test]
    fn test_name() {
        let policy = LruPolicy::<String, i32>::new();
        assert_eq!(policy.name(), "LRU");
    }
}
