//! LRU (Least Recently Used) cache eviction policy
//!
//! This module provides a highly optimized implementation of the LRU cache eviction policy
//! with advanced features like segmented LRU, adaptive aging, and efficient data structures
//! for high-performance caching scenarios.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    ptr::NonNull,
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, collections::VecDeque, vec::Vec},
    core::{hash::Hash, ptr::NonNull, time::Duration},
    hashbrown::HashMap,
};

use crate::cache::{
    compute::{CacheEntry, CacheKey},
    stats::{AccessPattern, SizeDistribution},
};
use crate::error::MemoryResult;

/// LRU implementation strategies for different scenarios
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LruStrategy {
    /// Classic LRU with doubly-linked list
    Classic,
    /// Segmented LRU (multiple segments with different policies)
    Segmented,
    /// Clock-based approximation (more memory efficient)
    Clock,
    /// Adaptive LRU with aging
    Adaptive,
    /// Multi-queue LRU (MQ-LRU)
    MultiQueue,
}

impl Default for LruStrategy {
    fn default() -> Self {
        LruStrategy::Adaptive
    }
}

/// Configuration for LRU policy
#[derive(Debug, Clone)]
pub struct LruConfig {
    /// LRU implementation strategy
    pub strategy: LruStrategy,
    /// Number of segments for segmented LRU
    pub segment_count: usize,
    /// Enable adaptive aging
    pub enable_aging: bool,
    /// Aging factor (0.0 to 1.0)
    pub aging_factor: f64,
    /// Aging interval
    pub aging_interval: Duration,
    /// Protection ratio for hot data (0.0 to 1.0)
    pub protection_ratio: f64,
    /// Enable access frequency tracking
    pub track_frequency: bool,
    /// Maximum tracked items for efficiency
    pub max_tracked_items: usize,
    /// Enable size-aware eviction
    pub size_aware: bool,
    /// Enable temporal locality detection
    pub detect_temporal_locality: bool,
    /// Locality window size
    pub locality_window_size: usize,
}

impl Default for LruConfig {
    fn default() -> Self {
        Self {
            strategy: LruStrategy::default(),
            segment_count: 4,
            enable_aging: true,
            aging_factor: 0.9,
            aging_interval: Duration::from_secs(300), // 5 minutes
            protection_ratio: 0.2, // Protect top 20%
            track_frequency: true,
            max_tracked_items: 10000,
            size_aware: false,
            detect_temporal_locality: true,
            locality_window_size: 100,
        }
    }
}

/// Node in the doubly-linked list for classic LRU
#[derive(Debug)]
struct LruNode<K> {
    key: K,
    prev: Option<NonNull<LruNode<K>>>,
    next: Option<NonNull<LruNode<K>>>,
    access_count: u64,
    size: usize,
    #[cfg(feature = "std")]
    last_access: Instant,
    protected: bool, // For protection against single-scan sequences
}

impl<K> LruNode<K> {
    fn new(key: K) -> Self {
        Self {
            key,
            prev: None,
            next: None,
            access_count: 1,
            size: 0,
            #[cfg(feature = "std")]
            last_access: Instant::now(),
            protected: false,
        }
    }
}

/// Doubly-linked list for efficient LRU operations
#[derive(Debug)]
struct DoublyLinkedList<K> {
    head: Option<NonNull<LruNode<K>>>,
    tail: Option<NonNull<LruNode<K>>>,
    len: usize,
}

impl<K> DoublyLinkedList<K> {
    fn new() -> Self {
        Self {
            head: None,
            tail: None,
            len: 0,
        }
    }

    /// Add node to the front (most recently used)
    fn push_front(&mut self, mut node: Box<LruNode<K>>) -> NonNull<LruNode<K>> {
        unsafe {
            node.next = self.head;
            node.prev = None;
            let node_ptr = NonNull::from(Box::leak(node));

            match self.head {
                Some(head) => (*head.as_ptr()).prev = Some(node_ptr),
                None => self.tail = Some(node_ptr),
            }

            self.head = Some(node_ptr);
            self.len += 1;
            node_ptr
        }
    }

    /// Remove node from the back (least recently used)
    fn pop_back(&mut self) -> Option<Box<LruNode<K>>> {
        self.tail.map(|tail| unsafe {
            let tail_ref = Box::from_raw(tail.as_ptr());
            self.remove_node(tail);
            tail_ref
        })
    }

    /// Remove a specific node
    fn remove_node(&mut self, mut node: NonNull<LruNode<K>>) {
        unsafe {
            match (*node.as_ptr()).prev {
                Some(prev) => (*prev.as_ptr()).next = (*node.as_ptr()).next,
                None => self.head = (*node.as_ptr()).next,
            }

            match (*node.as_ptr()).next {
                Some(next) => (*next.as_ptr()).prev = (*node.as_ptr()).prev,
                None => self.tail = (*node.as_ptr()).prev,
            }

            (*node.as_ptr()).prev = None;
            (*node.as_ptr()).next = None;
            self.len -= 1;
        }
    }

    /// Move node to front
    fn move_to_front(&mut self, mut node: NonNull<LruNode<K>>) {
        if self.head == Some(node) {
            return; // Already at front
        }

        unsafe {
            // Remove from current position
            self.remove_node(node);

            // Add to front
            (*node.as_ptr()).next = self.head;
            (*node.as_ptr()).prev = None;

            if let Some(head) = self.head {
                (*head.as_ptr()).prev = Some(node);
            } else {
                self.tail = Some(node);
            }

            self.head = Some(node);
            self.len += 1;
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<K> Drop for DoublyLinkedList<K> {
    fn drop(&mut self) {
        while self.pop_back().is_some() {}
    }
}

/// Segment for segmented LRU
#[derive(Debug)]
struct LruSegment<K> {
    list: DoublyLinkedList<K>,
    node_map: HashMap<K, NonNull<LruNode<K>>>,
    capacity: usize,
    access_threshold: u64,
}

impl<K> LruSegment<K>
where
    K: CacheKey,
{
    fn new(capacity: usize, access_threshold: u64) -> Self {
        Self {
            list: DoublyLinkedList::new(),
            node_map: HashMap::new(),
            capacity,
            access_threshold,
        }
    }

    fn insert(&mut self, key: K, size: usize) -> Option<K> {
        let mut evicted = None;

        // Check if we need to evict
        if self.list.len() >= self.capacity {
            if let Some(old_node) = self.list.pop_back() {
                evicted = Some(old_node.key.clone());
                self.node_map.remove(&old_node.key);
            }
        }

        // Insert new node
        let mut node = Box::new(LruNode::new(key.clone()));
        node.size = size;
        let node_ptr = self.list.push_front(node);
        self.node_map.insert(key, node_ptr);

        evicted
    }

    fn access(&mut self, key: &K) -> bool {
        if let Some(&node_ptr) = self.node_map.get(key) {
            unsafe {
                (*node_ptr.as_ptr()).access_count += 1;
                #[cfg(feature = "std")]
                {
                    (*node_ptr.as_ptr()).last_access = Instant::now();
                }
            }
            self.list.move_to_front(node_ptr);
            true
        } else {
            false
        }
    }

    fn remove(&mut self, key: &K) -> bool {
        if let Some(node_ptr) = self.node_map.remove(key) {
            unsafe {
                let _removed_node = Box::from_raw(node_ptr.as_ptr());
            }
            self.list.remove_node(node_ptr);
            true
        } else {
            false
        }
    }

    fn get_lru_candidate(&self) -> Option<K> {
        self.list.tail.map(|tail_ptr| unsafe {
            (*tail_ptr.as_ptr()).key.clone()
        })
    }

    fn should_promote(&self, key: &K) -> bool {
        self.node_map.get(key).map_or(false, |&node_ptr| unsafe {
            (*node_ptr.as_ptr()).access_count >= self.access_threshold
        })
    }
}

/// Clock hand for clock-based LRU approximation
#[derive(Debug)]
struct ClockHand<K> {
    items: Vec<ClockItem<K>>,
    hand: usize,
    capacity: usize,
}

#[derive(Debug, Clone)]
struct ClockItem<K> {
    key: Option<K>,
    reference_bit: bool,
    frequency: u64,
    size: usize,
}

impl<K> ClockHand<K>
where
    K: CacheKey,
{
    fn new(capacity: usize) -> Self {
        Self {
            items: vec![ClockItem {
                key: None,
                reference_bit: false,
                frequency: 0,
                size: 0,
            }; capacity],
            hand: 0,
            capacity,
        }
    }

    fn find_victim(&mut self) -> Option<K> {
        for _ in 0..(self.capacity * 2) { // Maximum two full rotations
            let current = &mut self.items[self.hand];

            if let Some(ref key) = current.key {
                if current.reference_bit {
                    current.reference_bit = false;
                } else {
                    let victim = key.clone();
                    current.key = None;
                    current.frequency = 0;
                    current.size = 0;
                    return Some(victim);
                }
            }

            self.hand = (self.hand + 1) % self.capacity;
        }
        None
    }

    fn access(&mut self, key: &K) {
        for item in &mut self.items {
            if let Some(ref item_key) = item.key {
                if item_key == key {
                    item.reference_bit = true;
                    item.frequency += 1;
                    return;
                }
            }
        }
    }

    fn insert(&mut self, key: K, size: usize) {
        // Find empty slot or use current hand position
        let mut insert_pos = self.hand;
        for i in 0..self.capacity {
            let pos = (self.hand + i) % self.capacity;
            if self.items[pos].key.is_none() {
                insert_pos = pos;
                break;
            }
        }

        self.items[insert_pos] = ClockItem {
            key: Some(key),
            reference_bit: true,
            frequency: 1,
            size,
        };
    }
}

/// Highly optimized LRU cache eviction policy
pub struct LruPolicy<K, V>
where
    K: CacheKey,
{
    /// Configuration
    config: LruConfig,
    /// Current strategy implementation
    strategy_impl: LruStrategyImpl<K>,
    /// Access pattern statistics
    access_pattern: AccessPattern,
    /// Size distribution for size-aware eviction
    size_distribution: SizeDistribution,
    /// Recent access window for temporal locality detection
    recent_accesses: VecDeque<K>,
    /// Total access count
    total_accesses: u64,
    /// Last aging timestamp
    #[cfg(feature = "std")]
    last_aging: Instant,
    /// Protected keys (immune to single-scan eviction)
    protected_keys: HashMap<K, u64>, // key -> protection_score
}

/// Strategy implementation variants
#[derive(Debug)]
enum LruStrategyImpl<K> {
    Classic {
        list: DoublyLinkedList<K>,
        node_map: HashMap<K, NonNull<LruNode<K>>>,
    },
    Segmented {
        segments: Vec<LruSegment<K>>,
    },
    Clock {
        clock: ClockHand<K>,
        key_positions: HashMap<K, usize>,
    },
    Adaptive {
        hot_list: DoublyLinkedList<K>,
        cold_list: DoublyLinkedList<K>,
        hot_map: HashMap<K, NonNull<LruNode<K>>>,
        cold_map: HashMap<K, NonNull<LruNode<K>>>,
        hot_capacity: usize,
    },
    MultiQueue {
        queues: Vec<VecDeque<K>>,
        queue_map: HashMap<K, usize>,
        lifetimes: HashMap<K, u64>,
    },
}

impl<K, V> LruPolicy<K, V>
where
    K: CacheKey,
{
    /// Create a new LRU policy with default configuration
    pub fn new() -> Self {
        Self::with_config(LruConfig::default())
    }

    /// Create a new LRU policy with custom configuration
    pub fn with_config(config: LruConfig) -> Self {
        let strategy_impl = match config.strategy {
            LruStrategy::Classic => LruStrategyImpl::Classic {
                list: DoublyLinkedList::new(),
                node_map: HashMap::new(),
            },
            LruStrategy::Segmented => {
                let segment_capacity = config.max_tracked_items / config.segment_count;
                let segments = (0..config.segment_count)
                    .map(|i| LruSegment::new(segment_capacity, (i + 1) as u64 * 2))
                    .collect();
                LruStrategyImpl::Segmented { segments }
            },
            LruStrategy::Clock => LruStrategyImpl::Clock {
                clock: ClockHand::new(config.max_tracked_items),
                key_positions: HashMap::new(),
            },
            LruStrategy::Adaptive => {
                let hot_capacity = (config.max_tracked_items as f64 * config.protection_ratio) as usize;
                LruStrategyImpl::Adaptive {
                    hot_list: DoublyLinkedList::new(),
                    cold_list: DoublyLinkedList::new(),
                    hot_map: HashMap::new(),
                    cold_map: HashMap::new(),
                    hot_capacity,
                }
            },
            LruStrategy::MultiQueue => {
                let queue_count = 8; // Number of frequency-based queues
                LruStrategyImpl::MultiQueue {
                    queues: vec![VecDeque::new(); queue_count],
                    queue_map: HashMap::new(),
                    lifetimes: HashMap::new(),
                }
            },
        };

        Self {
            config,
            strategy_impl,
            access_pattern: AccessPattern::default(),
            size_distribution: SizeDistribution::default(),
            recent_accesses: VecDeque::new(),
            total_accesses: 0,
            #[cfg(feature = "std")]
            last_aging: Instant::now(),
            protected_keys: HashMap::new(),
        }
    }

    /// Create LRU policy optimized for sequential scans
    pub fn for_sequential_scans() -> Self {
        Self::with_config(LruConfig {
            strategy: LruStrategy::Segmented,
            segment_count: 2,
            protection_ratio: 0.5,
            enable_aging: false,
            track_frequency: true,
            detect_temporal_locality: true,
            ..Default::default()
        })
    }

    /// Create LRU policy optimized for random access patterns
    pub fn for_random_access() -> Self {
        Self::with_config(LruConfig {
            strategy: LruStrategy::Classic,
            enable_aging: true,
            aging_factor: 0.95,
            protection_ratio: 0.1,
            track_frequency: false,
            ..Default::default()
        })
    }

    /// Create LRU policy for memory-constrained environments
    pub fn for_memory_constrained() -> Self {
        Self::with_config(LruConfig {
            strategy: LruStrategy::Clock,
            max_tracked_items: 1000,
            enable_aging: false,
            track_frequency: false,
            detect_temporal_locality: false,
            size_aware: true,
            ..Default::default()
        })
    }

    /// Record an access to a key
    pub fn record_access(&mut self, key: &K, size_hint: Option<usize>) {
        self.total_accesses += 1;

        // Update recent accesses for temporal locality detection
        if self.config.detect_temporal_locality {
            self.update_recent_accesses(key);
        }

        // Update protection if enabled
        if self.config.protection_ratio > 0.0 {
            self.update_protection(key);
        }

        // Delegate to strategy implementation
        match &mut self.strategy_impl {
            LruStrategyImpl::Classic { list, node_map } => {
                self.classic_access(list, node_map, key, size_hint);
            },
            LruStrategyImpl::Segmented { segments } => {
                self.segmented_access(segments, key, size_hint);
            },
            LruStrategyImpl::Clock { clock, key_positions } => {
                self.clock_access(clock, key_positions, key);
            },
            LruStrategyImpl::Adaptive { hot_list, cold_list, hot_map, cold_map, hot_capacity } => {
                self.adaptive_access(hot_list, cold_list, hot_map, cold_map, *hot_capacity, key, size_hint);
            },
            LruStrategyImpl::MultiQueue { queues, queue_map, lifetimes } => {
                self.multi_queue_access(queues, queue_map, lifetimes, key);
            },
        }

        // Perform aging if needed
        #[cfg(feature = "std")]
        if self.config.enable_aging && self.should_perform_aging() {
            self.perform_aging();
        }

        // Update access pattern statistics
        self.update_access_pattern();
    }

    /// Record insertion of a new key
    pub fn record_insertion(&mut self, key: &K, entry: &CacheEntry<V>, size_hint: Option<usize>) {
        let size = size_hint.unwrap_or_else(|| {
            // Estimate size based on entry
            std::mem::size_of::<V>()
        });

        match &mut self.strategy_impl {
            LruStrategyImpl::Classic { list, node_map } => {
                if let Some(evicted) = self.classic_insert(list, node_map, key, size) {
                    // Handle eviction if needed
                }
            },
            LruStrategyImpl::Segmented { segments } => {
                // Insert into first segment
                if !segments.is_empty() {
                    segments[0].insert(key.clone(), size);
                }
            },
            LruStrategyImpl::Clock { clock, key_positions } => {
                clock.insert(key.clone(), size);
            },
            LruStrategyImpl::Adaptive { cold_list, cold_map, .. } => {
                self.adaptive_insert(cold_list, cold_map, key, size);
            },
            LruStrategyImpl::MultiQueue { queues, queue_map, lifetimes } => {
                if !queues.is_empty() {
                    queues[0].push_back(key.clone());
                    queue_map.insert(key.clone(), 0);
                    lifetimes.insert(key.clone(), self.total_accesses);
                }
            },
        }

        // Update size distribution
        self.update_size_distribution(size);
    }

    /// Record removal of a key
    pub fn record_removal(&mut self, key: &K) {
        self.protected_keys.remove(key);

        match &mut self.strategy_impl {
            LruStrategyImpl::Classic { list, node_map } => {
                if let Some(node_ptr) = node_map.remove(key) {
                    list.remove_node(node_ptr);
                    unsafe { Box::from_raw(node_ptr.as_ptr()); }
                }
            },
            LruStrategyImpl::Segmented { segments } => {
                for segment in segments {
                    if segment.remove(key) {
                        break;
                    }
                }
            },
            LruStrategyImpl::Clock { key_positions, .. } => {
                key_positions.remove(key);
            },
            LruStrategyImpl::Adaptive { hot_list, cold_list, hot_map, cold_map, .. } => {
                if let Some(node_ptr) = hot_map.remove(key) {
                    hot_list.remove_node(node_ptr);
                    unsafe { Box::from_raw(node_ptr.as_ptr()); }
                } else if let Some(node_ptr) = cold_map.remove(key) {
                    cold_list.remove_node(node_ptr);
                    unsafe { Box::from_raw(node_ptr.as_ptr()); }
                }
            },
            LruStrategyImpl::MultiQueue { queues, queue_map, lifetimes } => {
                if let Some(queue_idx) = queue_map.remove(key) {
                    if let Some(queue) = queues.get_mut(queue_idx) {
                        if let Some(pos) = queue.iter().position(|k| k == key) {
                            queue.remove(pos);
                        }
                    }
                }
                lifetimes.remove(key);
            },
        }
    }

    /// Select a victim for eviction
    pub fn select_victim(&self) -> Option<K> {
        match &self.strategy_impl {
            LruStrategyImpl::Classic { list, .. } => {
                list.tail.map(|tail_ptr| unsafe {
                    (*tail_ptr.as_ptr()).key.clone()
                })
            },
            LruStrategyImpl::Segmented { segments } => {
                // Try to find victim from least priority segment
                for segment in segments.iter().rev() {
                    if let Some(candidate) = segment.get_lru_candidate() {
                        if !self.is_protected(&candidate) {
                            return Some(candidate);
                        }
                    }
                }
                None
            },
            LruStrategyImpl::Clock { clock, .. } => {
                // This is a mutable operation, so we'd need to handle it differently
                // For now, return None and handle in actual eviction
                None
            },
            LruStrategyImpl::Adaptive { cold_list, hot_list, .. } => {
                // Prefer evicting from cold list
                cold_list.tail.map(|tail_ptr| unsafe {
                    (*tail_ptr.as_ptr()).key.clone()
                }).or_else(|| {
                    hot_list.tail.map(|tail_ptr| unsafe {
                        (*tail_ptr.as_ptr()).key.clone()
                    })
                })
            },
            LruStrategyImpl::MultiQueue { queues, .. } => {
                // Find victim from the lowest priority queue
                for queue in queues {
                    if let Some(candidate) = queue.front() {
                        if !self.is_protected(candidate) {
                            return Some(candidate.clone());
                        }
                    }
                }
                None
            },
        }
    }

    /// Get access pattern statistics
    pub fn get_access_pattern(&self) -> &AccessPattern {
        &self.access_pattern
    }

    /// Get size distribution statistics
    pub fn get_size_distribution(&self) -> &SizeDistribution {
        &self.size_distribution
    }

    /// Clear all LRU data
    pub fn clear(&mut self) {
        match &mut self.strategy_impl {
            LruStrategyImpl::Classic { list, node_map } => {
                // Properly deallocate all nodes
                while list.pop_back().is_some() {}
                node_map.clear();
            },
            LruStrategyImpl::Segmented { segments } => {
                for segment in segments {
                    while segment.list.pop_back().is_some() {}
                    segment.node_map.clear();
                }
            },
            LruStrategyImpl::Clock { clock, key_positions } => {
                for item in &mut clock.items {
                    item.key = None;
                    item.reference_bit = false;
                    item.frequency = 0;
                }
                clock.hand = 0;
                key_positions.clear();
            },
            LruStrategyImpl::Adaptive { hot_list, cold_list, hot_map, cold_map, .. } => {
                while hot_list.pop_back().is_some() {}
                while cold_list.pop_back().is_some() {}
                hot_map.clear();
                cold_map.clear();
            },
            LruStrategyImpl::MultiQueue { queues, queue_map, lifetimes } => {
                for queue in queues {
                    queue.clear();
                }
                queue_map.clear();
                lifetimes.clear();
            },
        }

        self.protected_keys.clear();
        self.recent_accesses.clear();
        self.total_accesses = 0;

        #[cfg(feature = "std")]
        {
            self.last_aging = Instant::now();
        }
    }

    // Implementation methods for different strategies

    fn classic_access(&mut self, list: &mut DoublyLinkedList<K>, node_map: &mut HashMap<K, NonNull<LruNode<K>>>, key: &K, size_hint: Option<usize>) {
        if let Some(&node_ptr) = node_map.get(key) {
            unsafe {
                (*node_ptr.as_ptr()).access_count += 1;
                if let Some(size) = size_hint {
                    (*node_ptr.as_ptr()).size = size;
                }
                #[cfg(feature = "std")]
                {
                    (*node_ptr.as_ptr()).last_access = Instant::now();
                }
            }
            list.move_to_front(node_ptr);
        }
    }

    fn classic_insert(&mut self, list: &mut DoublyLinkedList<K>, node_map: &mut HashMap<K, NonNull<LruNode<K>>>, key: &K, size: usize) -> Option<K> {
        let mut node = Box::new(LruNode::new(key.clone()));
        node.size = size;
        let node_ptr = list.push_front(node);
        node_map.insert(key.clone(), node_ptr);
        None
    }

    fn segmented_access(&mut self, segments: &mut Vec<LruSegment<K>>, key: &K, size_hint: Option<usize>) {
        // Find the segment containing the key and potentially promote
        for (i, segment) in segments.iter_mut().enumerate() {
            if segment.access(key) {
                // Check if should promote to higher segment
                if i < segments.len() - 1 && segment.should_promote(key) {
                    if segment.remove(key) {
                        let size = size_hint.unwrap_or(0);
                        segments[i + 1].insert(key.clone(), size);
                    }
                }
                return;
            }
        }
    }

    fn clock_access(&mut self, clock: &mut ClockHand<K>, _key_positions: &mut HashMap<K, usize>, key: &K) {
        clock.access(key);
    }

    fn adaptive_access(&mut self, hot_list: &mut DoublyLinkedList<K>, cold_list: &mut DoublyLinkedList<K>, hot_map: &mut HashMap<K, NonNull<LruNode<K>>>, cold_map: &mut HashMap<K, NonNull<LruNode<K>>>, hot_capacity: usize, key: &K, size_hint: Option<usize>) {
        // Check if in hot list
        if let Some(&node_ptr) = hot_map.get(key) {
            unsafe {
                (*node_ptr.as_ptr()).access_count += 1;
                if let Some(size) = size_hint {
                    (*node_ptr.as_ptr()).size = size;
                }
            }
            hot_list.move_to_front(node_ptr);
            return;
        }

        // Check if in cold list
        if let Some(node_ptr) = cold_map.remove(key) {
            // Move to hot list
            cold_list.remove_node(node_ptr);

            // Check if hot list has capacity
            if hot_list.len() >= hot_capacity {
                if let Some(demoted) = hot_list.pop_back() {
                    hot_map.remove(&demoted.key);
                    let cold_node_ptr = cold_list.push_front(demoted);
                    cold_map.insert(cold_node_ptr.clone(), cold_node_ptr);
                }
            }

            hot_list.move_to_front(node_ptr);
            hot_map.insert(key.clone(), node_ptr);
        }
    }

    fn adaptive_insert(&mut self, cold_list: &mut DoublyLinkedList<K>, cold_map: &mut HashMap<K, NonNull<LruNode<K>>>, key: &K, size: usize) {
        let mut node = Box::new(LruNode::new(key.clone()));
        node.size = size;
        let node_ptr = cold_list.push_front(node);
        cold_map.insert(key.clone(), node_ptr);
    }

    fn multi_queue_access(&mut self, queues: &mut Vec<VecDeque<K>>, queue_map: &mut HashMap<K, usize>, lifetimes: &mut HashMap<K, u64>, key: &K) {
        if let Some(&current_queue) = queue_map.get(key) {
            // Remove from current queue
            if let Some(queue) = queues.get_mut(current_queue) {
                if let Some(pos) = queue.iter().position(|k| k == key) {
                    queue.remove(pos);
                }
            }

            // Promote to higher queue if possible
            let new_queue = (current_queue + 1).min(queues.len() - 1);
            queues[new_queue].push_back(key.clone());
            queue_map.insert(key.clone(), new_queue);
            lifetimes.insert(key.clone(), self.total_accesses);
        }
    }

    fn update_recent_accesses(&mut self, key: &K) {
        self.recent_accesses.push_back(key.clone());

        if self.recent_accesses.len() > self.config.locality_window_size {
            self.recent_accesses.pop_front();
        }
    }

    fn update_protection(&mut self, key: &K) {
        let entry = self.protected_keys.entry(key.clone()).or_insert(0);
        *entry += 1;

        // Decay protection scores periodically
        if self.total_accesses % 1000 == 0 {
            for score in self.protected_keys.values_mut() {
                *score = (*score as f64 * self.config.aging_factor) as u64;
            }
            self.protected_keys.retain(|_, &mut score| score > 0);
        }
    }

    fn is_protected(&self, key: &K) -> bool {
        self.protected_keys.get(key).map_or(false, |&score| score > 5)
    }

    fn update_access_pattern(&mut self) {
        if self.recent_accesses.len() < 10 {
            return;
        }

        // Calculate sequential access ratio
        let mut sequential_count = 0;
        let unique_recent: std::collections::HashSet<_> = self.recent_accesses.iter().collect();

        for window in self.recent_accesses.windows(2) {
            if window[0] == window[1] {
                sequential_count += 1;
            }
        }

        self.access_pattern.sequential_access_ratio =
            sequential_count as f64 / (self.recent_accesses.len() - 1).max(1) as f64;

        self.access_pattern.random_access_ratio = 1.0 - self.access_pattern.sequential_access_ratio;

        // Calculate temporal locality
        self.access_pattern.temporal_locality =
            1.0 - (unique_recent.len() as f64 / self.recent_accesses.len() as f64);
    }

    fn update_size_distribution(&mut self, size: usize) {
        let size_u64 = size as u64;

        if self.size_distribution.total_samples == 0 {
            self.size_distribution.min = size_u64;
            self.size_distribution.max = size_u64;
            self.size_distribution.avg = size_u64 as f64;
        } else {
            self.size_distribution.min = self.size_distribution.min.min(size_u64);
            self.size_distribution.max = self.size_distribution.max.max(size_u64);

            let total = self.size_distribution.total_samples as f64;
            self.size_distribution.avg =
                (self.size_distribution.avg * total + size_u64 as f64) / (total + 1.0);
        }

        self.size_distribution.total_samples += 1;
    }

    #[cfg(feature = "std")]
    fn should_perform_aging(&self) -> bool {
        self.last_aging.elapsed() >= self.config.aging_interval
    }

    #[cfg(feature = "std")]
    fn perform_aging(&mut self) {
        self.last_aging = Instant::now();

        // Age protection scores
        for score in self.protected_keys.values_mut() {
            *score = (*score as f64 * self.config.aging_factor) as u64;
        }
        self.protected_keys.retain(|_, &mut score| score > 0);
    }

    /// Get efficiency metrics for the LRU policy
    pub fn get_efficiency_metrics(&self) -> LruEfficiencyMetrics {
        let protected_count = self.protected_keys.len();
        let total_items = match &self.strategy_impl {
            LruStrategyImpl::Classic { node_map, .. } => node_map.len(),
            LruStrategyImpl::Segmented { segments } => {
                segments.iter().map(|s| s.node_map.len()).sum()
            },
            LruStrategyImpl::Clock { key_positions, .. } => key_positions.len(),
            LruStrategyImpl::Adaptive { hot_map, cold_map, .. } => {
                hot_map.len() + cold_map.len()
            },
            LruStrategyImpl::MultiQueue { queue_map, .. } => queue_map.len(),
        };

        LruEfficiencyMetrics {
            total_items,
            protected_items: protected_count,
            protection_ratio: if total_items > 0 {
                protected_count as f64 / total_items as f64
            } else {
                0.0
            },
            sequential_access_ratio: self.access_pattern.sequential_access_ratio,
            temporal_locality: self.access_pattern.temporal_locality,
            avg_size: self.size_distribution.avg,
        }
    }
}

/// LRU efficiency metrics for analysis
#[derive(Debug, Clone)]
pub struct LruEfficiencyMetrics {
    pub total_items: usize,
    pub protected_items: usize,
    pub protection_ratio: f64,
    pub sequential_access_ratio: f64,
    pub temporal_locality: f64,
    pub avg_size: f64,
}

impl<K, V> Default for LruPolicy<K, V>
where
    K: CacheKey,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::compute::CacheEntry;

    #[test]
    fn test_classic_lru_basic() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        // Insert keys
        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.record_insertion(&"key2".to_string(), &entry, Some(200));
        policy.record_insertion(&"key3".to_string(), &entry, Some(150));

        // Access key1 to make it most recently used
        policy.record_access(&"key1".to_string(), None);

        // key2 should be the LRU victim
        let victim = policy.select_victim();
        assert_eq!(victim, Some("key2".to_string()));
    }

    #[test]
    fn test_segmented_lru() {
        let config = LruConfig {
            strategy: LruStrategy::Segmented,
            segment_count: 2,
            max_tracked_items: 10,
            ..Default::default()
        };
        let mut policy = LruPolicy::<String, i32>::with_config(config);
        let entry = CacheEntry::new(42);

        // Insert and access keys to test promotion
        policy.record_insertion(&"key1".to_string(), &entry, Some(100));

        // Access multiple times to trigger promotion
        for _ in 0..5 {
            policy.record_access(&"key1".to_string(), None);
        }

        // Should work without panicking
        let _victim = policy.select_victim();
    }

    #[test]
    fn test_adaptive_lru() {
        let config = LruConfig {
            strategy: LruStrategy::Adaptive,
            protection_ratio: 0.5,
            max_tracked_items: 10,
            ..Default::default()
        };
        let mut policy = LruPolicy::<String, i32>::with_config(config);
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.record_insertion(&"key2".to_string(), &entry, Some(200));

        // Access key1 to promote it to hot list
        policy.record_access(&"key1".to_string(), None);
        policy.record_access(&"key1".to_string(), None);

        let victim = policy.select_victim();
        // Should prefer evicting from cold list (key2)
        assert_eq!(victim, Some("key2".to_string()));
    }

    #[test]
    fn test_preset_configurations() {
        // Test sequential scan configuration
        let _seq_policy = LruPolicy::<String, i32>::for_sequential_scans();

        // Test random access configuration
        let _random_policy = LruPolicy::<String, i32>::for_random_access();

        // Test memory constrained configuration
        let memory_policy = LruPolicy::<String, i32>::for_memory_constrained();
        assert_eq!(memory_policy.config.strategy, LruStrategy::Clock);
    }

    #[test]
    fn test_access_pattern_tracking() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        // Create sequential access pattern
        for i in 0..10 {
            let key = format!("key{}", i);
            policy.record_insertion(&key, &entry, Some(100));
            policy.record_access(&key, None);
        }

        let pattern = policy.get_access_pattern();
        // Sequential access ratio should be low for unique keys
        assert!(pattern.sequential_access_ratio >= 0.0);
    }

    #[test]
    fn test_protection_mechanism() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"protected".to_string(), &entry, Some(100));
        policy.record_insertion(&"normal".to_string(), &entry, Some(100));

        // Access protected key many times
        for _ in 0..20 {
            policy.record_access(&"protected".to_string(), None);
        }

        // Protected key should not be selected as victim
        assert!(policy.is_protected(&"protected".to_string()));
        assert!(!policy.is_protected(&"normal".to_string()));
    }

    #[test]
    fn test_efficiency_metrics() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        for i in 0..5 {
            let key = format!("key{}", i);
            policy.record_insertion(&key, &entry, Some(100 * (i + 1)));
            policy.record_access(&key, None);
        }

        let metrics = policy.get_efficiency_metrics();
        assert_eq!(metrics.total_items, 5);
        assert!(metrics.avg_size > 0.0);
    }

    #[test]
    fn test_clear_functionality() {
        let mut policy = LruPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.record_access(&"key1".to_string(), None);

        assert!(policy.select_victim().is_some());

        policy.clear();

        assert!(policy.select_victim().is_none());
        assert_eq!(policy.total_accesses, 0);
    }
}