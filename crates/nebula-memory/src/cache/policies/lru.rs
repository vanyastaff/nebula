//! Optimized LRU (Least Recently Used) cache eviction policy
//!
//! This module provides an O(1) LRU implementation using a HashMap with doubly-linked list indices.
//! When the cache is full, it evicts the least recently used entry.
//!
//! All operations (access, insertion, removal) are O(1) for optimal performance.

use std::{
    collections::HashMap,
    marker::PhantomData,
};

use crate::cache::compute::{CacheEntry, CacheKey};

use super::{EvictionEntry, EvictionPolicy, VictimSelector};

/// Node in the doubly-linked LRU list
#[derive(Debug, Clone)]
struct LruNode<K> {
    key: K,
    prev: Option<usize>,
    next: Option<usize>,
}

/// Optimized LRU (Least Recently Used) cache eviction policy
///
/// Maintains keys in a doubly-linked list ordered by recency of access. When an entry is accessed,
/// it's moved to the front in O(1) time. When eviction is needed, the key at the back
/// (least recently used) is selected in O(1) time.
///
/// This implementation uses a HashMap for O(1) lookups and a Vec-based doubly-linked list
/// for O(1) reordering. All operations are constant time.
///
/// # Performance
///
/// - Access: O(1)
/// - Insertion: O(1)
/// - Removal: O(1)
/// - Victim selection: O(1)
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
    /// Map from key to node index in the list
    key_to_index: HashMap<K, usize>,
    /// Doubly-linked list of nodes (Vec acts as arena allocator)
    nodes: Vec<LruNode<K>>,
    /// Index of the most recently used node (head)
    head: Option<usize>,
    /// Index of the least recently used node (tail)
    tail: Option<usize>,
    /// Free list for reusing removed node slots
    free_list: Vec<usize>,
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
            key_to_index: HashMap::new(),
            nodes: Vec::new(),
            head: None,
            tail: None,
            free_list: Vec::new(),
        }
    }

    /// Create a new LRU eviction policy with preallocated capacity
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            _phantom: PhantomData,
            key_to_index: HashMap::with_capacity(capacity),
            nodes: Vec::with_capacity(capacity),
            head: None,
            tail: None,
            free_list: Vec::new(),
        }
    }

    /// Get the number of tracked keys
    #[must_use]
    pub fn len(&self) -> usize {
        self.key_to_index.len()
    }

    /// Check if the policy is tracking any keys
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.key_to_index.is_empty()
    }

    /// Clear all tracked keys
    pub fn clear(&mut self) {
        self.key_to_index.clear();
        self.nodes.clear();
        self.head = None;
        self.tail = None;
        self.free_list.clear();
    }

    /// Move a node to the front (make it most recently used) - O(1)
    fn move_to_front(&mut self, index: usize) {
        if self.head == Some(index) {
            // Already at front
            return;
        }

        // Remove from current position
        let node = &self.nodes[index];
        let prev = node.prev;
        let next = node.next;

        if let Some(prev_idx) = prev {
            self.nodes[prev_idx].next = next;
        }
        if let Some(next_idx) = next {
            self.nodes[next_idx].prev = prev;
        }

        // Update tail if this was the tail
        if self.tail == Some(index) {
            self.tail = prev;
        }

        // Insert at front
        if let Some(old_head) = self.head {
            self.nodes[old_head].prev = Some(index);
        }

        self.nodes[index].prev = None;
        self.nodes[index].next = self.head;
        self.head = Some(index);

        // If this was the only node, it's also the tail
        if self.tail.is_none() {
            self.tail = Some(index);
        }
    }

    /// Add a new node at the front - O(1)
    fn push_front(&mut self, key: K) -> usize {
        let index = if let Some(free_idx) = self.free_list.pop() {
            // Reuse a free slot
            self.nodes[free_idx] = LruNode {
                key,
                prev: None,
                next: self.head,
            };
            free_idx
        } else {
            // Allocate new slot
            let index = self.nodes.len();
            self.nodes.push(LruNode {
                key,
                prev: None,
                next: self.head,
            });
            index
        };

        if let Some(old_head) = self.head {
            self.nodes[old_head].prev = Some(index);
        }

        self.head = Some(index);

        if self.tail.is_none() {
            self.tail = Some(index);
        }

        index
    }

    /// Remove a node from the list - O(1)
    fn remove_node(&mut self, index: usize) {
        let node = &self.nodes[index];
        let prev = node.prev;
        let next = node.next;

        if let Some(prev_idx) = prev {
            self.nodes[prev_idx].next = next;
        } else {
            // This was the head
            self.head = next;
        }

        if let Some(next_idx) = next {
            self.nodes[next_idx].prev = prev;
        } else {
            // This was the tail
            self.tail = prev;
        }

        // Add to free list for reuse
        self.free_list.push(index);
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
        // Move to front if key exists - O(1)
        if let Some(&index) = self.key_to_index.get(key) {
            self.move_to_front(index);
        }
    }

    fn record_insertion(&mut self, key: &K, _entry: &CacheEntry<V>) {
        // Add to front (most recently used) - O(1)
        let index = self.push_front(key.clone());
        self.key_to_index.insert(key.clone(), index);
    }

    fn record_removal(&mut self, key: &K) {
        // Remove from list - O(1)
        if let Some(index) = self.key_to_index.remove(key) {
            self.remove_node(index);
        }
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
        // Return the least recently used key (tail of list) - O(1)
        self.tail.map(|tail_idx| self.nodes[tail_idx].key.clone())
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
        let entries: Vec<EvictionEntry<'_, String, i32>> = vec![];

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
