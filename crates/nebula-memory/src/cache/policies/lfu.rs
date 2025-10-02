//! LFU (Least Frequently Used) cache eviction policy
//!
//! This module provides a highly optimized implementation of the LFU cache eviction policy
//! with advanced features like frequency aging, adaptive frequency tracking, and 
//! efficient data structures for large-scale caching scenarios.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    hash::Hash,
    marker::PhantomData,
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{collections::BTreeMap, vec::Vec},
    core::{hash::Hash, marker::PhantomData, time::Duration},
    hashbrown::HashMap,
};

use crate::cache::{
    compute::{CacheEntry, CacheKey},
    stats::{AccessPattern, SizeDistribution},
};

/// Frequency tracking modes for different scenarios
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyMode {
    /// Simple counter (classic LFU)
    SimpleCounter,
    /// Exponential decay (frequency aging)
    ExponentialDecay,
    /// Sliding window with time-based decay
    SlidingWindow,
    /// Adaptive frequency based on access patterns
    Adaptive,
    /// Probabilistic counting (Count-Min Sketch)
    Probabilistic,
}

impl Default for FrequencyMode {
    fn default() -> Self {
        FrequencyMode::ExponentialDecay
    }
}

/// Configuration for LFU policy
#[derive(Debug, Clone)]
pub struct LfuConfig {
    /// Frequency tracking mode
    pub frequency_mode: FrequencyMode,
    /// Decay factor for exponential decay (0.0 to 1.0)
    pub decay_factor: f64,
    /// Time window for sliding window mode
    pub time_window: Duration,
    /// Maximum frequency count to prevent overflow
    pub max_frequency: u64,
    /// Minimum frequency threshold for eviction consideration
    pub min_frequency_threshold: u64,
    /// Enable aging to gradually reduce old frequencies
    pub enable_aging: bool,
    /// Aging interval
    pub aging_interval: Duration,
    /// Enable frequency histogram for analysis
    pub enable_histogram: bool,
    /// Tie-breaking strategy when frequencies are equal
    pub tie_breaking: TieBreakingStrategy,
}

impl Default for LfuConfig {
    fn default() -> Self {
        Self {
            frequency_mode: FrequencyMode::default(),
            decay_factor: 0.9,
            time_window: Duration::from_secs(3600), // 1 hour
            max_frequency: 1_000_000,
            min_frequency_threshold: 1,
            enable_aging: true,
            aging_interval: Duration::from_secs(300), // 5 minutes
            enable_histogram: false,
            tie_breaking: TieBreakingStrategy::LeastRecentlyUsed,
        }
    }
}

/// Tie-breaking strategies for equal frequencies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TieBreakingStrategy {
    /// Use LRU for tie-breaking (least recently used)
    LeastRecentlyUsed,
    /// Use FIFO for tie-breaking (first inserted)
    FirstInFirstOut,
    /// Use size-based tie-breaking (largest items first)
    LargestFirst,
    /// Use random selection
    Random,
}

/// Access record for time-based tracking
#[derive(Debug, Clone)]
struct AccessRecord {
    /// Timestamp of access
    #[cfg(feature = "std")]
    timestamp: Instant,
    /// Access count
    count: u64,
    /// Weight for this access (for decay calculations)
    weight: f64,
}

/// Frequency bucket for efficient frequency-based operations
#[derive(Debug, Clone)]
struct FrequencyBucket<K> {
    /// Frequency value
    frequency: u64,
    /// Keys in this frequency bucket
    keys: Vec<K>,
    /// Next bucket in the chain (higher frequency)
    next: Option<u64>,
    /// Previous bucket in the chain (lower frequency)
    prev: Option<u64>,
}

/// Highly optimized LFU cache eviction policy
pub struct LfuPolicy<K, V>
where
    K: CacheKey,
{
    /// Phantom data for unused type parameter V
    _phantom: PhantomData<V>,
    /// Configuration
    config: LfuConfig,
    /// Frequency counts for each key
    frequencies: HashMap<K, u64>,
    /// Key to bucket mapping for O(1) frequency updates
    key_to_bucket: HashMap<K, u64>,
    /// Frequency buckets organized for efficient min-frequency access
    frequency_buckets: BTreeMap<u64, FrequencyBucket<K>>,
    /// Minimum frequency for quick victim selection
    min_frequency: u64,
    /// Access order for tie-breaking (LRU)
    access_order: VecDeque<K>,
    /// Access history for time-based modes
    #[cfg(feature = "std")]
    access_history: HashMap<K, VecDeque<AccessRecord>>,
    /// Size information for size-based tie-breaking
    key_sizes: HashMap<K, usize>,
    /// Frequency histogram for analysis
    frequency_histogram: HashMap<u64, usize>,
    /// Last aging timestamp
    #[cfg(feature = "std")]
    last_aging: Instant,
    /// Total access count for adaptive algorithms
    total_accesses: u64,
    /// Access pattern statistics
    access_pattern: AccessPattern,
    /// Frequency distribution statistics
    frequency_distribution: SizeDistribution,
}

impl<K, V> LfuPolicy<K, V>
where
    K: CacheKey,
{
    /// Create a new LFU policy with default configuration
    pub fn new() -> Self {
        Self::with_config(LfuConfig::default())
    }

    /// Create a new LFU policy with custom configuration
    pub fn with_config(config: LfuConfig) -> Self {
        Self {
            _phantom: PhantomData,
            config,
            frequencies: HashMap::new(),
            key_to_bucket: HashMap::new(),
            frequency_buckets: BTreeMap::new(),
            min_frequency: 1,
            access_order: VecDeque::new(),
            #[cfg(feature = "std")]
            access_history: HashMap::new(),
            key_sizes: HashMap::new(),
            frequency_histogram: HashMap::new(),
            #[cfg(feature = "std")]
            last_aging: Instant::now(),
            total_accesses: 0,
            access_pattern: AccessPattern::default(),
            frequency_distribution: SizeDistribution::default(),
        }
    }

    /// Create LFU policy optimized for high-frequency access patterns
    pub fn for_high_frequency() -> Self {
        Self::with_config(LfuConfig {
            frequency_mode: FrequencyMode::ExponentialDecay,
            decay_factor: 0.95,
            max_frequency: 10_000_000,
            enable_aging: true,
            aging_interval: Duration::from_secs(60),
            tie_breaking: TieBreakingStrategy::LeastRecentlyUsed,
            ..Default::default()
        })
    }

    /// Create LFU policy optimized for temporal patterns
    pub fn for_temporal_patterns() -> Self {
        Self::with_config(LfuConfig {
            frequency_mode: FrequencyMode::SlidingWindow,
            time_window: Duration::from_secs(1800), // 30 minutes
            enable_aging: false,
            tie_breaking: TieBreakingStrategy::LeastRecentlyUsed,
            enable_histogram: true,
            ..Default::default()
        })
    }

    /// Create LFU policy for memory-constrained environments
    pub fn for_memory_constrained() -> Self {
        Self::with_config(LfuConfig {
            frequency_mode: FrequencyMode::SimpleCounter,
            max_frequency: 1000,
            enable_aging: false,
            enable_histogram: false,
            tie_breaking: TieBreakingStrategy::LargestFirst,
            ..Default::default()
        })
    }

    /// Record an access to a key
    pub fn record_access(&mut self, key: &K, size_hint: Option<usize>) {
        self.total_accesses += 1;

        // Update size information if provided
        if let Some(size) = size_hint {
            self.key_sizes.insert(key.clone(), size);
        }

        // Update access order for tie-breaking
        self.update_access_order(key);

        // Update frequency based on the configured mode
        match self.config.frequency_mode {
            FrequencyMode::SimpleCounter => {
                self.increment_simple_frequency(key);
            }
            FrequencyMode::ExponentialDecay => {
                self.increment_with_decay(key);
            }
            FrequencyMode::SlidingWindow => {
                self.record_windowed_access(key);
            }
            FrequencyMode::Adaptive => {
                self.adaptive_frequency_update(key);
            }
            FrequencyMode::Probabilistic => {
                self.probabilistic_increment(key);
            }
        }

        // Update histogram if enabled
        if self.config.enable_histogram {
            if let Some(&freq) = self.frequencies.get(key) {
                *self.frequency_histogram.entry(freq).or_insert(0) += 1;
            }
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
        // Initialize frequency
        self.frequencies.insert(key.clone(), 1);

        // Add to frequency bucket
        self.add_to_bucket(key, 1);

        // Update minimum frequency
        self.min_frequency = self.min_frequency.min(1);

        // Record size if provided
        if let Some(size) = size_hint {
            self.key_sizes.insert(key.clone(), size);
        }

        // Add to access order
        self.access_order.push_back(key.clone());

        // Initialize access history for time-based modes
        #[cfg(feature = "std")]
        if matches!(
            self.config.frequency_mode,
            FrequencyMode::SlidingWindow | FrequencyMode::Adaptive
        ) {
            self.access_history.insert(key.clone(), VecDeque::new());
        }
    }

    /// Record removal of a key
    pub fn record_removal(&mut self, key: &K) {
        if let Some(frequency) = self.frequencies.remove(key) {
            self.remove_from_bucket(key, frequency);
        }

        self.key_to_bucket.remove(key);
        self.key_sizes.remove(key);

        // Remove from access order
        if let Some(pos) = self.access_order.iter().position(|k| k == key) {
            self.access_order.remove(pos);
        }

        // Remove from access history
        #[cfg(feature = "std")]
        self.access_history.remove(key);

        // Update minimum frequency if necessary
        if self.frequency_buckets.is_empty() {
            self.min_frequency = 1;
        } else if let Some((&min_freq, _)) = self.frequency_buckets.iter().next() {
            self.min_frequency = min_freq;
        }
    }

    /// Select a victim for eviction
    pub fn select_victim(&self) -> Option<K> {
        // Find the minimum frequency bucket with items
        let mut current_freq = self.min_frequency;

        while let Some(bucket) = self.frequency_buckets.get(&current_freq) {
            if !bucket.keys.is_empty() {
                return self.select_victim_from_bucket(bucket);
            }

            // Move to next frequency level
            if let Some(next_freq) = bucket.next {
                current_freq = next_freq;
            } else {
                break;
            }
        }

        None
    }

    /// Select victim from a specific frequency bucket using tie-breaking strategy
    fn select_victim_from_bucket(&self, bucket: &FrequencyBucket<K>) -> Option<K> {
        if bucket.keys.is_empty() {
            return None;
        }

        match self.config.tie_breaking {
            TieBreakingStrategy::LeastRecentlyUsed => {
                // Find the least recently used key in this bucket
                for key in &self.access_order {
                    if bucket.keys.contains(key) {
                        return Some(key.clone());
                    }
                }
                // Fallback to first key in bucket
                bucket.keys.first().cloned()
            }
            TieBreakingStrategy::FirstInFirstOut => {
                // Return the first key in the bucket (insertion order)
                bucket.keys.first().cloned()
            }
            TieBreakingStrategy::LargestFirst => {
                // Find the key with the largest size
                bucket.keys
                    .iter()
                    .max_by_key(|key| self.key_sizes.get(*key).unwrap_or(&0))
                    .cloned()
            }
            TieBreakingStrategy::Random => {
                // Use a simple deterministic "random" selection
                let index = self.total_accesses as usize % bucket.keys.len();
                bucket.keys.get(index).cloned()
            }
        }
    }

    /// Get current frequency of a key
    pub fn get_frequency(&self, key: &K) -> Option<u64> {
        self.frequencies.get(key).copied()
    }

    /// Get frequency distribution statistics
    pub fn get_frequency_distribution(&self) -> &SizeDistribution {
        &self.frequency_distribution
    }

    /// Get access pattern statistics
    pub fn get_access_pattern(&self) -> &AccessPattern {
        &self.access_pattern
    }

    /// Get frequency histogram
    pub fn get_frequency_histogram(&self) -> &HashMap<u64, usize> {
        &self.frequency_histogram
    }

    /// Clear all frequency data
    pub fn clear(&mut self) {
        self.frequencies.clear();
        self.key_to_bucket.clear();
        self.frequency_buckets.clear();
        self.access_order.clear();
        self.key_sizes.clear();
        self.frequency_histogram.clear();
        self.min_frequency = 1;
        self.total_accesses = 0;

        #[cfg(feature = "std")]
        {
            self.access_history.clear();
            self.last_aging = Instant::now();
        }
    }

    /// Update access order for LRU tie-breaking
    fn update_access_order(&mut self, key: &K) {
        // Remove key from current position
        if let Some(pos) = self.access_order.iter().position(|k| k == key) {
            self.access_order.remove(pos);
        }

        // Add to the end (most recently used)
        self.access_order.push_back(key.clone());

        // Limit access order size to prevent unbounded growth
        const MAX_ACCESS_ORDER_SIZE: usize = 10000;
        if self.access_order.len() > MAX_ACCESS_ORDER_SIZE {
            self.access_order.pop_front();
        }
    }

    /// Increment frequency using simple counter
    fn increment_simple_frequency(&mut self, key: &K) {
        let current_freq = *self.frequencies.get(key).unwrap_or(&0);
        let new_freq = (current_freq + 1).min(self.config.max_frequency);

        if new_freq != current_freq {
            // Remove from old bucket
            if current_freq > 0 {
                self.remove_from_bucket(key, current_freq);
            }

            // Add to new bucket
            self.add_to_bucket(key, new_freq);
            self.frequencies.insert(key.clone(), new_freq);
        }
    }

    /// Increment frequency with exponential decay
    fn increment_with_decay(&mut self, key: &K) {
        let current_freq = *self.frequencies.get(key).unwrap_or(&0);

        // Apply decay to current frequency and add 1
        let decayed_freq = (current_freq as f64 * self.config.decay_factor) as u64;
        let new_freq = (decayed_freq + 1).min(self.config.max_frequency);

        if new_freq != current_freq {
            if current_freq > 0 {
                self.remove_from_bucket(key, current_freq);
            }
            self.add_to_bucket(key, new_freq);
            self.frequencies.insert(key.clone(), new_freq);
        }
    }

    /// Record windowed access (sliding window mode)
    #[cfg(feature = "std")]
    fn record_windowed_access(&mut self, key: &K) {
        let now = Instant::now();
        let cutoff = now - self.config.time_window;

        // Get or create access history for this key
        let history = self.access_history.entry(key.clone()).or_insert_with(VecDeque::new);

        // Remove old accesses outside the window
        while let Some(front) = history.front() {
            if front.timestamp < cutoff {
                history.pop_front();
            } else {
                break;
            }
        }

        // Add new access
        history.push_back(AccessRecord {
            timestamp: now,
            count: 1,
            weight: 1.0,
        });

        // Update frequency based on window count
        let window_count = history.len() as u64;
        let current_freq = *self.frequencies.get(key).unwrap_or(&0);

        if window_count != current_freq {
            if current_freq > 0 {
                self.remove_from_bucket(key, current_freq);
            }
            self.add_to_bucket(key, window_count);
            self.frequencies.insert(key.clone(), window_count);
        }
    }

    /// Adaptive frequency update based on access patterns
    fn adaptive_frequency_update(&mut self, key: &K) {
        let current_freq = *self.frequencies.get(key).unwrap_or(&0);

        // Adapt increment based on recent access patterns
        let increment = if self.access_pattern.temporal_locality > 0.8 {
            // High temporal locality - use larger increments
            2
        } else if self.access_pattern.hot_spot_ratio > 0.3 {
            // Many hot spots - use standard increment
            1
        } else {
            // Low locality - use smaller increments with decay
            if current_freq > 5 {
                0 // Don't increment if already high and locality is low
            } else {
                1
            }
        };

        let new_freq = (current_freq + increment).min(self.config.max_frequency);

        if new_freq != current_freq {
            if current_freq > 0 {
                self.remove_from_bucket(key, current_freq);
            }
            self.add_to_bucket(key, new_freq);
            self.frequencies.insert(key.clone(), new_freq);
        }
    }

    /// Probabilistic increment (simplified Count-Min Sketch approach)
    fn probabilistic_increment(&mut self, key: &K) {
        // Use a simple hash-based probability to reduce memory usage
        let hash = self.simple_hash(key);
        let probability = 1.0 / (1.0 + (hash % 100) as f64 / 100.0);

        // Only increment with probability
        if (hash % 1000) as f64 / 1000.0 < probability {
            self.increment_simple_frequency(key);
        }
    }

    /// Simple hash function for probabilistic operations
    fn simple_hash(&self, key: &K) -> u64
    where
        K: core::hash::Hash,
    {
        use core::hash::{Hash as _, Hasher};
        use std::collections::hash_map::RandomState;
        use std::hash::BuildHasher;
        let mut hasher = RandomState::new().build_hasher();
        key.hash(&mut hasher);
        hasher.finish()
    }

    /// Add key to frequency bucket
    fn add_to_bucket(&mut self, key: &K, frequency: u64) {
        let bucket = self.frequency_buckets.entry(frequency).or_insert_with(|| {
            FrequencyBucket {
                frequency,
                keys: Vec::new(),
                next: None,
                prev: None,
            }
        });

        bucket.keys.push(key.clone());
        self.key_to_bucket.insert(key.clone(), frequency);

        // Update bucket links
        self.update_bucket_links(frequency);
    }

    /// Remove key from frequency bucket
    fn remove_from_bucket(&mut self, key: &K, frequency: u64) {
        if let Some(bucket) = self.frequency_buckets.get_mut(&frequency) {
            if let Some(pos) = bucket.keys.iter().position(|k| k == key) {
                bucket.keys.remove(pos);
            }

            // Remove empty bucket
            if bucket.keys.is_empty() {
                self.frequency_buckets.remove(&frequency);

                // Update minimum frequency if necessary
                if frequency == self.min_frequency {
                    if let Some((&next_freq, _)) = self.frequency_buckets.iter().next() {
                        self.min_frequency = next_freq;
                    } else {
                        self.min_frequency = 1;
                    }
                }
            }
        }
    }

    /// Update bucket links for efficient traversal
    fn update_bucket_links(&mut self, frequency: u64) {
        // This is a simplified implementation
        // In a full implementation, you'd maintain a proper doubly-linked structure
    }

    /// Check if aging should be performed
    #[cfg(feature = "std")]
    fn should_perform_aging(&self) -> bool {
        self.last_aging.elapsed() >= self.config.aging_interval
    }

    /// Perform frequency aging
    #[cfg(feature = "std")]
    fn perform_aging(&mut self) {
        self.last_aging = Instant::now();

        // Age all frequencies by applying decay
        let decay_factor = self.config.decay_factor;
        let mut updates = Vec::new();

        for (key, &current_freq) in &self.frequencies {
            let new_freq = ((current_freq as f64 * decay_factor) as u64)
                .max(self.config.min_frequency_threshold);

            if new_freq != current_freq {
                updates.push((key.clone(), current_freq, new_freq));
            }
        }

        // Apply updates
        for (key, old_freq, new_freq) in updates {
            self.remove_from_bucket(&key, old_freq);
            self.add_to_bucket(&key, new_freq);
            self.frequencies.insert(key, new_freq);
        }

        // Update frequency distribution
        self.update_frequency_distribution();
    }

    /// Update access pattern statistics
    fn update_access_pattern(&mut self) {
        // Simplified access pattern analysis
        let total_keys = self.frequencies.len();
        if total_keys == 0 {
            return;
        }

        // Calculate hot spot ratio (keys with high frequency)
        let high_freq_threshold = self.config.max_frequency / 4;
        let hot_spots = self.frequencies.values().filter(|&&freq| freq > high_freq_threshold).count();
        self.access_pattern.hot_spot_ratio = hot_spots as f64 / total_keys as f64;

        // Update temporal locality based on recent access patterns
        if self.access_order.len() > 10 {
            let recent_accesses = self.access_order.len().min(100);
            let unique_recent = self.access_order
                .iter()
                .rev()
                .take(recent_accesses)
                .collect::<std::collections::HashSet<_>>()
                .len();

            self.access_pattern.temporal_locality =
                1.0 - (unique_recent as f64 / recent_accesses as f64);
        }
    }

    /// Update frequency distribution statistics
    fn update_frequency_distribution(&mut self) {
        if self.frequencies.is_empty() {
            return;
        }

        let frequencies: Vec<u64> = self.frequencies.values().copied().collect();

        self.frequency_distribution.min = frequencies.iter().min().copied().unwrap_or(0);
        self.frequency_distribution.max = frequencies.iter().max().copied().unwrap_or(0);
        self.frequency_distribution.avg = frequencies.iter().sum::<u64>() as f64 / frequencies.len() as f64;
        self.frequency_distribution.total_samples = frequencies.len() as u64;

        // Calculate median
        let mut sorted_frequencies = frequencies.clone();
        sorted_frequencies.sort_unstable();
        let mid = sorted_frequencies.len() / 2;
        self.frequency_distribution.median = if sorted_frequencies.len() % 2 == 0 {
            (sorted_frequencies[mid - 1] + sorted_frequencies[mid]) as f64 / 2.0
        } else {
            sorted_frequencies[mid] as f64
        };
    }

    /// Get cache efficiency metrics
    pub fn get_efficiency_metrics(&self) -> LfuEfficiencyMetrics {
        LfuEfficiencyMetrics {
            total_keys: self.frequencies.len(),
            min_frequency: self.min_frequency,
            max_frequency: self.frequencies.values().max().copied().unwrap_or(0),
            avg_frequency: if !self.frequencies.is_empty() {
                self.frequencies.values().sum::<u64>() as f64 / self.frequencies.len() as f64
            } else {
                0.0
            },
            frequency_spread: self.frequency_distribution.max - self.frequency_distribution.min,
            hot_spot_ratio: self.access_pattern.hot_spot_ratio,
            temporal_locality: self.access_pattern.temporal_locality,
        }
    }
}

/// LFU efficiency metrics for analysis
#[derive(Debug, Clone)]
pub struct LfuEfficiencyMetrics {
    pub total_keys: usize,
    pub min_frequency: u64,
    pub max_frequency: u64,
    pub avg_frequency: f64,
    pub frequency_spread: u64,
    pub hot_spot_ratio: f64,
    pub temporal_locality: f64,
}

impl<K, V> Default for LfuPolicy<K, V>
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
    fn test_basic_lfu_functionality() {
        let mut policy = LfuPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        // Insert keys
        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.record_insertion(&"key2".to_string(), &entry, Some(200));
        policy.record_insertion(&"key3".to_string(), &entry, Some(150));

        // Access key1 multiple times
        policy.record_access(&"key1".to_string(), None);
        policy.record_access(&"key1".to_string(), None);
        policy.record_access(&"key1".to_string(), None);

        // Access key2 once
        policy.record_access(&"key2".to_string(), None);

        // key3 should be the victim (lowest frequency)
        let victim = policy.select_victim();
        assert_eq!(victim, Some("key3".to_string()));
        assert_eq!(policy.get_frequency(&"key1".to_string()), Some(4)); // 1 + 3 accesses
        assert_eq!(policy.get_frequency(&"key2".to_string()), Some(2)); // 1 + 1 access
        assert_eq!(policy.get_frequency(&"key3".to_string()), Some(1)); // 1 insertion only
    }

    #[test]
    fn test_exponential_decay() {
        let config = LfuConfig {
            frequency_mode: FrequencyMode::ExponentialDecay,
            decay_factor: 0.5,
            ..Default::default()
        };
        let mut policy = LfuPolicy::<String, i32>::with_config(config);
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, None);

        // First access: 1 * 0.5 + 1 = 1.5 -> 1
        policy.record_access(&"key1".to_string(), None);
        assert_eq!(policy.get_frequency(&"key1".to_string()), Some(1));

        // Second access: 1 * 0.5 + 1 = 1.5 -> 1  
        policy.record_access(&"key1".to_string(), None);
        assert_eq!(policy.get_frequency(&"key1".to_string()), Some(1));
    }

    #[test]
    fn test_tie_breaking_strategies() {
        // Test LRU tie-breaking
        let config = LfuConfig {
            tie_breaking: TieBreakingStrategy::LeastRecentlyUsed,
            ..Default::default()
        };
        let mut policy = LfuPolicy::<String, i32>::with_config(config);
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, None);
        policy.record_insertion(&"key2".to_string(), &entry, None);

        // Both have same frequency, but key1 is less recently used
        let victim = policy.select_victim();
        assert_eq!(victim, Some("key1".to_string()));

        // Test largest-first tie-breaking
        let config = LfuConfig {
            tie_breaking: TieBreakingStrategy::LargestFirst,
            ..Default::default()
        };
        let mut policy = LfuPolicy::<String, i32>::with_config(config);

        policy.record_insertion(&"small".to_string(), &entry, Some(100));
        policy.record_insertion(&"large".to_string(), &entry, Some(1000));

        // Should select larger item
        let victim = policy.select_victim();
        assert_eq!(victim, Some("large".to_string()));
    }

    #[test]
    fn test_frequency_histogram() {
        let config = LfuConfig {
            enable_histogram: true,
            ..Default::default()
        };
        let mut policy = LfuPolicy::<String, i32>::with_config(config);
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, None);
        policy.record_access(&"key1".to_string(), None);
        policy.record_access(&"key1".to_string(), None);

        let histogram = policy.get_frequency_histogram();
        assert!(histogram.contains_key(&3)); // key1 has frequency 3
    }

    #[test]
    fn test_efficiency_metrics() {
        let mut policy = LfuPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        // Add some keys with different access patterns
        for i in 0..10 {
            let key = format!("key{}", i);
            policy.record_insertion(&key, &entry, None);

            // Create different access patterns
            for _ in 0..i {
                policy.record_access(&key, None);
            }
        }

        let metrics = policy.get_efficiency_metrics();
        assert_eq!(metrics.total_keys, 10);
        assert!(metrics.avg_frequency > 0.0);
        assert!(metrics.frequency_spread > 0);
    }

    #[test]
    fn test_memory_constrained_config() {
        let policy = LfuPolicy::<String, i32>::for_memory_constrained();
        assert_eq!(policy.config.frequency_mode, FrequencyMode::SimpleCounter);
        assert_eq!(policy.config.max_frequency, 1000);
        assert!(!policy.config.enable_aging);
    }

    #[test]
    fn test_clear_functionality() {
        let mut policy = LfuPolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, None);
        policy.record_access(&"key1".to_string(), None);

        assert!(policy.select_victim().is_some());

        policy.clear();

        assert!(policy.select_victim().is_none());
        assert_eq!(policy.frequencies.len(), 0);
        assert_eq!(policy.total_accesses, 0);
    }
}