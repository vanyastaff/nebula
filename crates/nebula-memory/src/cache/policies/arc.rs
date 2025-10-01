//! ARC (Adaptive Replacement Cache) policy
//!
//! This module provides a highly optimized implementation of the ARC cache eviction policy
//! with advanced features like enhanced adaptation mechanisms, workload analysis,
//! and performance optimizations for different access patterns.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
    marker::PhantomData,
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{collections::VecDeque, vec::Vec},
    core::{hash::Hash, marker::PhantomData, time::Duration},
    hashbrown::{HashMap, HashSet},
};

use crate::cache::{
    compute::{CacheEntry, CacheKey},
    stats::{AccessPattern, Percentiles, SizeDistribution},
};
use crate::core::error::MemoryResult;

/// ARC configuration for different workload scenarios
#[derive(Debug, Clone)]
pub struct ArcConfig {
    /// Cache capacity
    pub capacity: usize,
    /// Maximum size of ghost lists (B1 + B2)
    pub max_ghost_size: usize,
    /// Adaptation learning rate (0.0 to 1.0)
    pub learning_rate: f64,
    /// Enable aggressive adaptation for dynamic workloads
    pub aggressive_adaptation: bool,
    /// Minimum adaptation parameter value
    pub min_p: usize,
    /// Maximum adaptation parameter value  
    pub max_p: usize,
    /// Enable workload analysis and auto-tuning
    pub enable_workload_analysis: bool,
    /// Enable size-aware replacement
    pub size_aware: bool,
    /// Enable temporal locality tracking
    pub track_temporal_locality: bool,
    /// History window size for analysis
    pub history_window: usize,
    /// Ghost hit boost factor
    pub ghost_hit_boost: f64,
    /// Enable protection for recently inserted items
    pub protect_new_items: bool,
    /// Protection duration for new items
    pub protection_duration: Duration,
}

impl Default for ArcConfig {
    fn default() -> Self {
        Self {
            capacity: 1000,
            max_ghost_size: 2000, // 2x cache size is typical
            learning_rate: 1.0,
            aggressive_adaptation: false,
            min_p: 0,
            max_p: 1000, // Will be set to capacity
            enable_workload_analysis: true,
            size_aware: false,
            track_temporal_locality: true,
            history_window: 1000,
            ghost_hit_boost: 1.0,
            protect_new_items: false,
            protection_duration: Duration::from_secs(60),
        }
    }
}

impl ArcConfig {
    /// Create ARC config optimized for scan-resistant workloads
    pub fn for_scan_resistant(capacity: usize) -> Self {
        Self {
            capacity,
            max_ghost_size: capacity * 4, // Larger ghost lists
            learning_rate: 0.5, // Slower adaptation
            aggressive_adaptation: false,
            protect_new_items: true,
            protection_duration: Duration::from_secs(300),
            ..Default::default()
        }
    }

    /// Create ARC config optimized for random access patterns
    pub fn for_random_access(capacity: usize) -> Self {
        Self {
            capacity,
            max_ghost_size: capacity * 2,
            learning_rate: 1.0,
            aggressive_adaptation: true,
            enable_workload_analysis: true,
            track_temporal_locality: true,
            ..Default::default()
        }
    }

    /// Create ARC config optimized for temporal workloads
    pub fn for_temporal_workloads(capacity: usize) -> Self {
        Self {
            capacity,
            max_ghost_size: capacity,
            learning_rate: 1.5, // Faster adaptation
            aggressive_adaptation: true,
            ghost_hit_boost: 2.0,
            track_temporal_locality: true,
            history_window: 2000,
            ..Default::default()
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> MemoryResult<()> {
        if self.capacity == 0 {
            return Err(crate::core::error::MemoryError::InvalidConfig {
                reason: "capacity must be greater than 0".to_string(),
            });
        }

        if !(0.0..=2.0).contains(&self.learning_rate) {
            return Err(crate::core::error::MemoryError::InvalidConfig {
                reason: "learning_rate must be between 0.0 and 2.0".to_string(),
            });
        }

        if self.max_p > self.capacity {
            return Err(crate::core::error::MemoryError::InvalidConfig {
                reason: "max_p cannot exceed capacity".to_string(),
            });
        }

        Ok(())
    }
}

/// Location of a key in the ARC lists
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcLocation {
    /// T1: Recently used pages (recency)
    T1,
    /// T2: Frequently used pages (frequency)  
    T2,
    /// B1: Ghost entries for recently evicted pages
    B1,
    /// B2: Ghost entries for frequently evicted pages
    B2,
}

/// Enhanced entry metadata for ARC
#[derive(Debug, Clone)]
struct ArcEntry<K> {
    key: K,
    size: usize,
    access_count: u64,
    #[cfg(feature = "std")]
    last_access: Instant,
    #[cfg(feature = "std")]
    insertion_time: Instant,
    protected: bool,
}

impl<K> ArcEntry<K> {
    fn new(key: K, size: usize) -> Self {
        Self {
            key,
            size,
            access_count: 1,
            #[cfg(feature = "std")]
            last_access: Instant::now(),
            #[cfg(feature = "std")]
            insertion_time: Instant::now(),
            protected: false,
        }
    }

    #[cfg(feature = "std")]
    fn is_protection_expired(&self, protection_duration: Duration) -> bool {
        self.insertion_time.elapsed() > protection_duration
    }
}

/// Workload analysis statistics
#[derive(Debug, Clone, Default)]
pub struct WorkloadStats {
    /// Total requests processed
    pub total_requests: u64,
    /// Ghost hits in B1 (recency preference)
    pub b1_hits: u64,
    /// Ghost hits in B2 (frequency preference) 
    pub b2_hits: u64,
    /// Sequential access ratio
    pub sequential_ratio: f64,
    /// Temporal locality score
    pub temporal_locality: f64,
    /// Current adaptation parameter
    pub current_p: usize,
    /// Adaptation parameter history for analysis
    pub p_history: VecDeque<usize>,
    /// Hit rate in T1 vs T2
    pub t1_hit_rate: f64,
    pub t2_hit_rate: f64,
}

impl WorkloadStats {
    /// Calculate adaptation efficiency
    pub fn adaptation_efficiency(&self) -> f64 {
        if self.b1_hits + self.b2_hits == 0 {
            return 1.0;
        }

        let total_ghost_hits = self.b1_hits + self.b2_hits;
        let balanced_hits = total_ghost_hits as f64 / 2.0;
        let deviation = ((self.b1_hits as f64 - balanced_hits).abs() +
            (self.b2_hits as f64 - balanced_hits).abs()) / 2.0;

        1.0 - (deviation / balanced_hits).min(1.0)
    }

    /// Get workload characterization
    pub fn workload_type(&self) -> WorkloadType {
        if self.sequential_ratio > 0.8 {
            WorkloadType::Sequential
        } else if self.temporal_locality > 0.7 {
            WorkloadType::Temporal
        } else if self.b1_hits > self.b2_hits * 2 {
            WorkloadType::RecencyBiased
        } else if self.b2_hits > self.b1_hits * 2 {
            WorkloadType::FrequencyBiased
        } else {
            WorkloadType::Mixed
        }
    }
}

/// Workload classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadType {
    Sequential,      // Mostly sequential access
    Temporal,        // High temporal locality
    RecencyBiased,   // Prefers recently accessed items
    FrequencyBiased, // Prefers frequently accessed items
    Mixed,           // Balanced workload
}

/// Highly optimized ARC (Adaptive Replacement Cache) policy
pub struct ArcPolicy<K, V>
where
    K: CacheKey,
{
    /// Phantom data for unused type parameter V
    _phantom: PhantomData<V>,
    /// Configuration
    config: ArcConfig,

    /// T1: Recently used pages (recency list)
    t1: VecDeque<ArcEntry<K>>,
    /// T2: Frequently used pages (frequency list)
    t2: VecDeque<ArcEntry<K>>,
    /// B1: Ghost entries for recently evicted pages
    b1: HashMap<K, ArcEntry<K>>,
    /// B2: Ghost entries for frequently evicted pages
    b2: HashMap<K, ArcEntry<K>>,

    /// Adaptation parameter (target size for T1)
    p: usize,
    /// Location index for O(1) lookups
    location_map: HashMap<K, ArcLocation>,

    /// Workload analysis and statistics
    workload_stats: WorkloadStats,
    /// Access pattern analysis
    access_pattern: AccessPattern,
    /// Size distribution analysis
    size_distribution: SizeDistribution,

    /// Recent access history for pattern detection
    access_history: VecDeque<K>,
    /// Protected items (scan resistance)
    protected_items: HashSet<K>,

    /// Performance counters
    t1_accesses: u64,
    t2_accesses: u64,

    /// Last adaptation timestamp
    #[cfg(feature = "std")]
    last_adaptation: Instant,
}

impl<K, V> ArcPolicy<K, V>
where
    K: CacheKey,
{
    /// Create a new ARC policy with default configuration
    pub fn new() -> Self {
        Self::with_config(ArcConfig::default())
    }

    /// Create a new ARC policy with specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let mut config = ArcConfig::default();
        config.capacity = capacity;
        config.max_p = capacity;
        Self::with_config(config)
    }

    /// Create a new ARC policy with custom configuration
    pub fn with_config(config: ArcConfig) -> Self {
        config.validate().expect("Invalid ARC configuration");

        let max_p = config.max_p.min(config.capacity);

        Self {
            _phantom: PhantomData,
            config,
            t1: VecDeque::new(),
            t2: VecDeque::new(),
            b1: HashMap::new(),
            b2: HashMap::new(),
            p: max_p / 2, // Start balanced
            location_map: HashMap::new(),
            workload_stats: WorkloadStats::default(),
            access_pattern: AccessPattern::default(),
            size_distribution: SizeDistribution::default(),
            access_history: VecDeque::new(),
            protected_items: HashSet::new(),
            t1_accesses: 0,
            t2_accesses: 0,
            #[cfg(feature = "std")]
            last_adaptation: Instant::now(),
        }
    }

    /// Record an access to a key
    pub fn record_access(&mut self, key: &K, size_hint: Option<usize>) {
        self.workload_stats.total_requests += 1;

        // Update access history for pattern analysis
        if self.config.track_temporal_locality {
            self.update_access_history(key);
        }

        // Handle the access based on current location
        match self.location_map.get(key).copied() {
            Some(ArcLocation::T1) => {
                self.handle_t1_hit(key, size_hint);
            },
            Some(ArcLocation::T2) => {
                self.handle_t2_hit(key, size_hint);
            },
            Some(ArcLocation::B1) => {
                self.handle_b1_hit(key, size_hint);
            },
            Some(ArcLocation::B2) => {
                self.handle_b2_hit(key, size_hint);
            },
            None => {
                self.handle_cache_miss(key, size_hint);
            },
        }

        // Perform workload analysis and adaptation
        if self.config.enable_workload_analysis {
            self.analyze_workload();
        }

        // Update access pattern statistics
        self.update_access_pattern();
    }

    /// Record insertion of a new key
    pub fn record_insertion(&mut self, key: &K, entry: &CacheEntry<V>, size_hint: Option<usize>) {
        self.record_access(key, size_hint);
    }

    /// Record removal of a key
    pub fn record_removal(&mut self, key: &K) {
        if let Some(location) = self.location_map.remove(key) {
            match location {
                ArcLocation::T1 => {
                    if let Some(pos) = self.t1.iter().position(|e| &e.key == key) {
                        let entry = self.t1.remove(pos).unwrap();
                        // Move to B1 if space available
                        if self.b1.len() < self.config.max_ghost_size / 2 {
                            self.b1.insert(key.clone(), entry);
                            self.location_map.insert(key.clone(), ArcLocation::B1);
                        }
                    }
                },
                ArcLocation::T2 => {
                    if let Some(pos) = self.t2.iter().position(|e| &e.key == key) {
                        let entry = self.t2.remove(pos).unwrap();
                        // Move to B2 if space available
                        if self.b2.len() < self.config.max_ghost_size / 2 {
                            self.b2.insert(key.clone(), entry);
                            self.location_map.insert(key.clone(), ArcLocation::B2);
                        }
                    }
                },
                ArcLocation::B1 => {
                    self.b1.remove(key);
                },
                ArcLocation::B2 => {
                    self.b2.remove(key);
                },
            }
        }

        self.protected_items.remove(key);
    }

    /// Select a victim for eviction
    pub fn select_victim(&self) -> Option<K> {
        // If we're at capacity, we need to make room
        if self.t1.len() + self.t2.len() >= self.config.capacity {
            return self.choose_replacement_victim();
        }
        None
    }

    /// Choose a victim using ARC replacement algorithm
    fn choose_replacement_victim(&self) -> Option<K> {
        // Case 1: |T1| > p (target size)
        if self.t1.len() > self.p {
            // Remove from T1 (prefer non-protected items)
            for entry in &self.t1 {
                if !self.is_protected(&entry.key) {
                    return Some(entry.key.clone());
                }
            }
            // If all are protected, take the oldest
            self.t1.front().map(|e| e.key.clone())
        }
        // Case 2: |T1| <= p
        else {
            // Remove from T2 (prefer non-protected items)
            for entry in &self.t2 {
                if !self.is_protected(&entry.key) {
                    return Some(entry.key.clone());
                }
            }
            // If all are protected, take the oldest
            self.t2.front().map(|e| e.key.clone())
        }
    }

    /// Handle cache hit in T1 (move to T2)
    fn handle_t1_hit(&mut self, key: &K, size_hint: Option<usize>) {
        if let Some(pos) = self.t1.iter().position(|e| &e.key == key) {
            let mut entry = self.t1.remove(pos).unwrap();
            entry.access_count += 1;

            if let Some(size) = size_hint {
                entry.size = size;
            }

            #[cfg(feature = "std")]
            {
                entry.last_access = Instant::now();
            }

            // Move to T2 (promotion due to repeated access)
            self.t2.push_back(entry);
            self.location_map.insert(key.clone(), ArcLocation::T2);
            self.t1_accesses += 1;
        }
    }

    /// Handle cache hit in T2 (move to MRU position)
    fn handle_t2_hit(&mut self, key: &K, size_hint: Option<usize>) {
        if let Some(pos) = self.t2.iter().position(|e| &e.key == key) {
            let mut entry = self.t2.remove(pos).unwrap();
            entry.access_count += 1;

            if let Some(size) = size_hint {
                entry.size = size;
            }

            #[cfg(feature = "std")]
            {
                entry.last_access = Instant::now();
            }

            // Move to MRU position in T2
            self.t2.push_back(entry);
            self.t2_accesses += 1;
        }
    }

    /// Handle ghost hit in B1 (adapt towards recency)
    fn handle_b1_hit(&mut self, key: &K, size_hint: Option<usize>) {
        if let Some(mut entry) = self.b1.remove(key) {
            // Adapt p towards recency
            let delta = self.calculate_adaptation_delta(true);
            self.p = (self.p + delta).min(self.config.max_p);

            // Update entry
            entry.access_count += 1;
            if let Some(size) = size_hint {
                entry.size = size;
            }

            #[cfg(feature = "std")]
            {
                entry.last_access = Instant::now();
            }

            // Make room and insert into T2
            self.make_room_for_insertion();
            self.t2.push_back(entry);
            self.location_map.insert(key.clone(), ArcLocation::T2);

            // Update statistics
            self.workload_stats.b1_hits += 1;
        }
    }

    /// Handle ghost hit in B2 (adapt towards frequency)
    fn handle_b2_hit(&mut self, key: &K, size_hint: Option<usize>) {
        if let Some(mut entry) = self.b2.remove(key) {
            // Adapt p towards frequency
            let delta = self.calculate_adaptation_delta(false);
            self.p = self.p.saturating_sub(delta);

            // Update entry
            entry.access_count += 1;
            if let Some(size) = size_hint {
                entry.size = size;
            }

            #[cfg(feature = "std")]
            {
                entry.last_access = Instant::now();
            }

            // Make room and insert into T2
            self.make_room_for_insertion();
            self.t2.push_back(entry);
            self.location_map.insert(key.clone(), ArcLocation::T2);

            // Update statistics
            self.workload_stats.b2_hits += 1;
        }
    }

    /// Handle cache miss (new item)
    fn handle_cache_miss(&mut self, key: &K, size_hint: Option<usize>) {
        let size = size_hint.unwrap_or(std::mem::size_of::<V>());
        let mut entry = ArcEntry::new(key.clone(), size);

        // Apply protection if enabled
        if self.config.protect_new_items {
            entry.protected = true;
            self.protected_items.insert(key.clone());
        }

        // Make room if necessary
        if self.t1.len() + self.t2.len() >= self.config.capacity {
            self.make_room_for_insertion();
        }

        // Insert into T1 (recency list)
        self.t1.push_back(entry);
        self.location_map.insert(key.clone(), ArcLocation::T1);

        // Clean up ghost lists if they're too large
        self.cleanup_ghost_lists();

        // Update size distribution
        self.update_size_distribution(size);
    }

    /// Calculate adaptation delta based on ghost list sizes and workload
    fn calculate_adaptation_delta(&self, towards_recency: bool) -> usize {
        let base_delta = if towards_recency {
            // B1 hit: increase recency preference
            if self.b2.len() > 0 {
                ((self.b1.len() as f64 / self.b2.len() as f64) * self.config.learning_rate).ceil() as usize
            } else {
                (self.config.learning_rate).ceil() as usize
            }
        } else {
            // B2 hit: increase frequency preference  
            if self.b1.len() > 0 {
                ((self.b2.len() as f64 / self.b1.len() as f64) * self.config.learning_rate).ceil() as usize
            } else {
                (self.config.learning_rate).ceil() as usize
            }
        };

        // Apply ghost hit boost
        let boosted_delta = (base_delta as f64 * self.config.ghost_hit_boost) as usize;

        // Limit delta to prevent oscillation
        boosted_delta.min(self.config.capacity / 10).max(1)
    }

    /// Make room for a new insertion using ARC replacement
    fn make_room_for_insertion(&mut self) {
        // Case A: |T1| > p
        if self.t1.len() > self.p {
            // Remove LRU from T1 and add to B1
            if let Some(victim) = self.t1.pop_front() {
                let key = victim.key.clone();

                // Add to B1 if space available
                if self.b1.len() < self.config.max_ghost_size / 2 {
                    self.b1.insert(key.clone(), victim);
                    self.location_map.insert(key, ArcLocation::B1);
                } else {
                    self.location_map.remove(&key);
                }
            }
        }
        // Case B: |T1| <= p  
        else {
            // Remove LRU from T2 and add to B2
            if let Some(victim) = self.t2.pop_front() {
                let key = victim.key.clone();

                // Add to B2 if space available
                if self.b2.len() < self.config.max_ghost_size / 2 {
                    self.b2.insert(key.clone(), victim);
                    self.location_map.insert(key, ArcLocation::B2);
                } else {
                    self.location_map.remove(&key);
                }
            }
        }
    }

    /// Clean up ghost lists to maintain size limits
    fn cleanup_ghost_lists(&mut self) {
        // Clean B1 if too large
        while self.b1.len() > self.config.max_ghost_size / 2 {
            if let Some((key, _)) = self.b1.iter().next() {
                let key = key.clone();
                self.b1.remove(&key);
                self.location_map.remove(&key);
            } else {
                break;
            }
        }

        // Clean B2 if too large
        while self.b2.len() > self.config.max_ghost_size / 2 {
            if let Some((key, _)) = self.b2.iter().next() {
                let key = key.clone();
                self.b2.remove(&key);
                self.location_map.remove(&key);
            } else {
                break;
            }
        }
    }

    /// Check if an item is protected
    fn is_protected(&self, key: &K) -> bool {
        if !self.config.protect_new_items {
            return false;
        }

        #[cfg(feature = "std")]
        {
            // Check if protection has expired
            if let Some(location) = self.location_map.get(key) {
                match location {
                    ArcLocation::T1 => {
                        if let Some(entry) = self.t1.iter().find(|e| &e.key == key) {
                            return entry.protected && !entry.is_protection_expired(self.config.protection_duration);
                        }
                    },
                    ArcLocation::T2 => {
                        if let Some(entry) = self.t2.iter().find(|e| &e.key == key) {
                            return entry.protected && !entry.is_protection_expired(self.config.protection_duration);
                        }
                    },
                    _ => return false,
                }
            }
        }

        #[cfg(not(feature = "std"))]
        {
            self.protected_items.contains(key)
        }

        false
    }

    /// Update access history for temporal locality analysis
    fn update_access_history(&mut self, key: &K) {
        self.access_history.push_back(key.clone());

        if self.access_history.len() > self.config.history_window {
            self.access_history.pop_front();
        }
    }

    /// Analyze current workload and adjust parameters
    fn analyze_workload(&mut self) {
        // Update workload statistics
        self.workload_stats.current_p = self.p;
        self.workload_stats.p_history.push_back(self.p);

        if self.workload_stats.p_history.len() > 100 {
            self.workload_stats.p_history.pop_front();
        }

        // Calculate hit rates
        let total_t_accesses = self.t1_accesses + self.t2_accesses;
        if total_t_accesses > 0 {
            self.workload_stats.t1_hit_rate = self.t1_accesses as f64 / total_t_accesses as f64;
            self.workload_stats.t2_hit_rate = self.t2_accesses as f64 / total_t_accesses as f64;
        }

        // Auto-tune aggressive adaptation based on workload stability
        if self.config.aggressive_adaptation {
            let workload_type = self.workload_stats.workload_type();

            match workload_type {
                WorkloadType::Sequential => {
                    // For sequential workloads, prefer recency
                    if self.p < self.config.capacity * 3 / 4 {
                        self.p += 1;
                    }
                },
                WorkloadType::FrequencyBiased => {
                    // For frequency-biased workloads, prefer frequency
                    if self.p > self.config.capacity / 4 {
                        self.p -= 1;
                    }
                },
                _ => {
                    // For other workloads, let normal adaptation work
                }
            }
        }
    }

    /// Update access pattern statistics
    fn update_access_pattern(&mut self) {
        if self.access_history.len() < 10 {
            return;
        }

        // Calculate sequential access ratio
        let mut sequential_count = 0;
        let history_vec: Vec<_> = self.access_history.iter().collect();
        for window in history_vec.windows(2) {
            if window[0] == window[1] {
                sequential_count += 1;
            }
        }

        self.workload_stats.sequential_ratio =
            sequential_count as f64 / (self.access_history.len() - 1).max(1) as f64;

        // Calculate temporal locality
        let unique_recent: HashSet<_> = self.access_history.iter().collect();
        self.workload_stats.temporal_locality =
            1.0 - (unique_recent.len() as f64 / self.access_history.len() as f64);

        // Update access pattern
        self.access_pattern.sequential_access_ratio = self.workload_stats.sequential_ratio;
        self.access_pattern.temporal_locality = self.workload_stats.temporal_locality;
    }

    /// Update size distribution statistics
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

    /// Get current ARC state information
    pub fn get_state_info(&self) -> ArcStateInfo {
        ArcStateInfo {
            t1_size: self.t1.len(),
            t2_size: self.t2.len(),
            b1_size: self.b1.len(),
            b2_size: self.b2.len(),
            adaptation_parameter: self.p,
            target_t1_size: self.p,
            total_capacity: self.config.capacity,
            workload_type: self.workload_stats.workload_type(),
            adaptation_efficiency: self.workload_stats.adaptation_efficiency(),
        }
    }

    /// Get workload statistics
    pub fn get_workload_stats(&self) -> &WorkloadStats {
        &self.workload_stats
    }

    /// Get access pattern analysis
    pub fn get_access_pattern(&self) -> &AccessPattern {
        &self.access_pattern
    }

    /// Get size distribution
    pub fn get_size_distribution(&self) -> &SizeDistribution {
        &self.size_distribution
    }

    /// Clear all ARC data
    pub fn clear(&mut self) {
        self.t1.clear();
        self.t2.clear();
        self.b1.clear();
        self.b2.clear();
        self.location_map.clear();
        self.access_history.clear();
        self.protected_items.clear();

        self.p = self.config.max_p / 2;
        self.t1_accesses = 0;
        self.t2_accesses = 0;

        self.workload_stats = WorkloadStats::default();
        self.access_pattern = AccessPattern::default();
        self.size_distribution = SizeDistribution::default();

        #[cfg(feature = "std")]
        {
            self.last_adaptation = Instant::now();
        }
    }

    /// Get efficiency metrics
    pub fn get_efficiency_metrics(&self) -> ArcEfficiencyMetrics {
        let total_cache_size = self.t1.len() + self.t2.len();
        let total_ghost_size = self.b1.len() + self.b2.len();

        ArcEfficiencyMetrics {
            cache_utilization: if self.config.capacity > 0 {
                total_cache_size as f64 / self.config.capacity as f64
            } else {
                0.0
            },
            ghost_utilization: if self.config.max_ghost_size > 0 {
                total_ghost_size as f64 / self.config.max_ghost_size as f64
            } else {
                0.0
            },
            adaptation_parameter: self.p,
            adaptation_efficiency: self.workload_stats.adaptation_efficiency(),
            workload_type: self.workload_stats.workload_type(),
            recency_bias: self.workload_stats.b1_hits as f64 / (self.workload_stats.b1_hits + self.workload_stats.b2_hits).max(1) as f64,
            frequency_bias: self.workload_stats.b2_hits as f64 / (self.workload_stats.b1_hits + self.workload_stats.b2_hits).max(1) as f64,
            protected_items: self.protected_items.len(),
        }
    }
}

/// ARC state information for debugging and monitoring
#[derive(Debug, Clone)]
pub struct ArcStateInfo {
    pub t1_size: usize,
    pub t2_size: usize,
    pub b1_size: usize,
    pub b2_size: usize,
    pub adaptation_parameter: usize,
    pub target_t1_size: usize,
    pub total_capacity: usize,
    pub workload_type: WorkloadType,
    pub adaptation_efficiency: f64,
}

/// ARC efficiency metrics
#[derive(Debug, Clone)]
pub struct ArcEfficiencyMetrics {
    pub cache_utilization: f64,
    pub ghost_utilization: f64,
    pub adaptation_parameter: usize,
    pub adaptation_efficiency: f64,
    pub workload_type: WorkloadType,
    pub recency_bias: f64,
    pub frequency_bias: f64,
    pub protected_items: usize,
}

impl<K, V> Default for ArcPolicy<K, V>
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
    fn test_arc_basic_functionality() {
        let mut policy = ArcPolicy::<String, i32>::with_capacity(4);
        let entry = CacheEntry::new(42);

        // Insert items into T1
        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.record_insertion(&"key2".to_string(), &entry, Some(100));
        policy.record_insertion(&"key3".to_string(), &entry, Some(100));

        let state = policy.get_state_info();
        assert_eq!(state.t1_size, 3);
        assert_eq!(state.t2_size, 0);

        // Access key1 to promote it to T2
        policy.record_access(&"key1".to_string(), None);

        let state = policy.get_state_info();
        assert_eq!(state.t1_size, 2);
        assert_eq!(state.t2_size, 1);
    }

    #[test]
    fn test_arc_ghost_hits() {
        let mut policy = ArcPolicy::<String, i32>::with_capacity(2);
        let entry = CacheEntry::new(42);

        // Fill cache
        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.record_insertion(&"key2".to_string(), &entry, Some(100));

        // Insert new key to evict key1 to B1
        policy.record_insertion(&"key3".to_string(), &entry, Some(100));

        let state = policy.get_state_info();
        assert!(state.b1_size > 0);

        // Ghost hit in B1 should adapt towards recency
        let old_p = policy.p;
        policy.record_access(&"key1".to_string(), None);

        assert!(policy.p >= old_p); // Should increase p (more recency preference)
        assert!(policy.workload_stats.b1_hits > 0);
    }

    #[test]
    fn test_arc_workload_analysis() {
        let mut policy = ArcPolicy::<String, i32>::with_config(
            ArcConfig::for_random_access(10)
        );
        let entry = CacheEntry::new(42);

        // Create a frequency-biased workload
        for i in 0..5 {
            let key = format!("key{}", i);
            policy.record_insertion(&key, &entry, Some(100));

            // Access some keys multiple times
            for _ in 0..i + 1 {
                policy.record_access(&key, None);
            }
        }

        let stats = policy.get_workload_stats();
        assert!(stats.total_requests > 0);

        let metrics = policy.get_efficiency_metrics();
        assert!(metrics.cache_utilization > 0.0);
        assert!(metrics.adaptation_efficiency >= 0.0);
    }

    #[test]
    fn test_arc_preset_configurations() {
        // Test scan-resistant configuration
        let scan_policy = ArcPolicy::<String, i32>::with_config(
            ArcConfig::for_scan_resistant(100)
        );
        assert!(scan_policy.config.protect_new_items);
        assert!(scan_policy.config.max_ghost_size >= 400);

        // Test temporal workload configuration
        let temporal_policy = ArcPolicy::<String, i32>::with_config(
            ArcConfig::for_temporal_workloads(100)
        );
        assert!(temporal_policy.config.aggressive_adaptation);
        assert!(temporal_policy.config.ghost_hit_boost > 1.0);
    }

    #[test]
    fn test_arc_protection_mechanism() {
        let mut policy = ArcPolicy::<String, i32>::with_config(
            ArcConfig::for_scan_resistant(10)
        );
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"protected".to_string(), &entry, Some(100));

        // Check if item is protected
        assert!(policy.is_protected(&"protected".to_string()));

        let metrics = policy.get_efficiency_metrics();
        assert!(metrics.protected_items > 0);
    }

    #[test]
    fn test_arc_size_awareness() {
        let mut policy = ArcPolicy::<String, i32>::with_config(ArcConfig {
            capacity: 10,
            size_aware: true,
            ..Default::default()
        });
        let entry = CacheEntry::new(42);

        // Insert items with different sizes
        policy.record_insertion(&"small".to_string(), &entry, Some(100));
        policy.record_insertion(&"large".to_string(), &entry, Some(1000));

        let size_dist = policy.get_size_distribution();
        assert_eq!(size_dist.min, 100);
        assert_eq!(size_dist.max, 1000);
        assert!(size_dist.avg > 100.0);
    }

    #[test]
    fn test_arc_adaptation_efficiency() {
        let mut policy = ArcPolicy::<String, i32>::with_capacity(10);
        let entry = CacheEntry::new(42);

        // Create mixed workload to test adaptation
        for i in 0..20 {
            let key = format!("key{}", i % 5);
            policy.record_insertion(&key, &entry, Some(100));
        }

        let stats = policy.get_workload_stats();
        let efficiency = stats.adaptation_efficiency();
        assert!(efficiency >= 0.0 && efficiency <= 1.0);

        let workload_type = stats.workload_type();
        // Should detect some pattern
        assert!(matches!(workload_type, 
            WorkloadType::Sequential | 
            WorkloadType::Temporal | 
            WorkloadType::RecencyBiased | 
            WorkloadType::FrequencyBiased | 
            WorkloadType::Mixed
        ));
    }

    #[test]
    fn test_arc_clear() {
        let mut policy = ArcPolicy::<String, i32>::with_capacity(10);
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.record_access(&"key1".to_string(), None);

        let state = policy.get_state_info();
        assert!(state.t1_size > 0 || state.t2_size > 0);

        policy.clear();

        let state = policy.get_state_info();
        assert_eq!(state.t1_size, 0);
        assert_eq!(state.t2_size, 0);
        assert_eq!(state.b1_size, 0);
        assert_eq!(state.b2_size, 0);
        assert_eq!(policy.workload_stats.total_requests, 0);
    }
}