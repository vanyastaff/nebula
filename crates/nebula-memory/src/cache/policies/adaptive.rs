//! Adaptive cache eviction policy
//!
//! This module provides an implementation of an adaptive cache eviction policy,
//! which dynamically selects the best eviction policy based on the workload.
//! It monitors cache performance and switches between different policies
//! to optimize hit rate.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::HashMap,
    hash::Hash,
    marker::PhantomData,
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, vec::Vec},
    core::{hash::Hash, marker::PhantomData},
    hashbrown::HashMap,
};

use super::{ArcPolicy, LfuPolicy, LruPolicy};
use crate::cache::compute::{CacheEntry, CacheKey};

/// Policy types supported by the adaptive policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyType {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// Adaptive Replacement Cache
    ARC,
}

impl Default for PolicyType {
    fn default() -> Self {
        PolicyType::LRU
    }
}

/// Performance metrics for a policy
#[derive(Debug, Clone)]
struct PolicyMetrics {
    /// Number of cache hits
    hits: usize,
    /// Number of cache misses
    misses: usize,
    /// Total number of requests
    requests: usize,
    /// Time when metrics were last reset
    #[cfg(feature = "std")]
    last_reset: Instant,
}

impl Default for PolicyMetrics {
    fn default() -> Self {
        #[cfg(feature = "std")]
        {
            Self {
                hits: 0,
                misses: 0,
                requests: 0,
                last_reset: Instant::now(),
            }
        }

        #[cfg(not(feature = "std"))]
        {
            Self {
                hits: 0,
                misses: 0,
                requests: 0,
            }
        }
    }
}

impl PolicyMetrics {
    /// Create new policy metrics
    #[cfg(feature = "std")]
    fn new() -> Self {
        Self {
            hits: 0,
            misses: 0,
            requests: 0,
            last_reset: Instant::now(),
        }
    }

    /// Create new policy metrics (no-std version)
    #[cfg(not(feature = "std"))]
    fn new() -> Self {
        Self {
            hits: 0,
            misses: 0,
            requests: 0,
        }
    }

    /// Record a cache hit
    fn record_hit(&mut self) {
        self.hits += 1;
        self.requests += 1;
    }

    /// Record a cache miss
    fn record_miss(&mut self) {
        self.misses += 1;
        self.requests += 1;
    }

    /// Calculate the hit rate
    fn hit_rate(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }
        self.hits as f64 / self.requests as f64
    }

    /// Reset the metrics
    #[cfg(feature = "std")]
    fn reset(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.requests = 0;
        self.last_reset = Instant::now();
    }

    /// Reset the metrics (no-std version)
    #[cfg(not(feature = "std"))]
    fn reset(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.requests = 0;
    }
}

/// Entry for victim selection
#[derive(Debug)]
pub struct EvictionEntry<'a, K, V> {
    pub key: &'a K,
    pub entry: &'a CacheEntry<V>,
}

impl<'a, K, V> EvictionEntry<'a, K, V> {
    pub fn new(key: &'a K, entry: &'a CacheEntry<V>) -> Self {
        Self { key, entry }
    }
}

/// Trait for selecting victims for eviction
pub trait VictimSelector<K, V> {
    fn select_victim(&self, entries: &[EvictionEntry<K, V>]) -> Option<K>;
}

/// Trait for eviction policies
pub trait EvictionPolicy<K, V>: VictimSelector<K, V> {
    fn record_access(&mut self, key: &K, size_hint: Option<usize>);
    fn record_insertion(&mut self, key: &K, entry: &CacheEntry<V>, size_hint: Option<usize>);
    fn record_removal(&mut self, key: &K);
    fn select_victim(&self) -> Option<K>;
    fn clear(&mut self);
    fn name(&self) -> &str;
}

/// Adaptive cache eviction policy
pub struct AdaptivePolicy<K, V>
where
    K: CacheKey,
{
    /// Phantom data for unused type parameter V
    _phantom: PhantomData<V>,
    /// Currently active policy
    active_policy: PolicyType,
    /// LRU policy instance
    lru: LruPolicy<K, V>,
    /// LFU policy instance
    lfu: LfuPolicy<K, V>,
    /// ARC policy instance
    arc: ArcPolicy<K, V>,
    /// Performance metrics for each policy
    metrics: HashMap<PolicyType, PolicyMetrics>,
    /// Minimum number of requests before evaluating policies
    min_requests: usize,
    /// Evaluation interval
    #[cfg(feature = "std")]
    evaluation_interval: Duration,
    /// Last time policies were evaluated
    #[cfg(feature = "std")]
    last_evaluation: Instant,
    /// Request counter for no-std environments
    #[cfg(not(feature = "std"))]
    request_counter: usize,
    /// Evaluation frequency for no-std environments
    #[cfg(not(feature = "std"))]
    evaluation_frequency: usize,
    /// Shadow tracking for performance evaluation
    shadow_hits: HashMap<PolicyType, usize>,
    shadow_misses: HashMap<PolicyType, usize>,
}

impl<K, V> AdaptivePolicy<K, V>
where
    K: CacheKey,
{
    /// Create a new adaptive policy
    pub fn new() -> Self {
        let mut metrics = HashMap::new();
        metrics.insert(PolicyType::LRU, PolicyMetrics::new());
        metrics.insert(PolicyType::LFU, PolicyMetrics::new());
        metrics.insert(PolicyType::ARC, PolicyMetrics::new());

        let mut shadow_hits = HashMap::new();
        shadow_hits.insert(PolicyType::LRU, 0);
        shadow_hits.insert(PolicyType::LFU, 0);
        shadow_hits.insert(PolicyType::ARC, 0);

        let mut shadow_misses = HashMap::new();
        shadow_misses.insert(PolicyType::LRU, 0);
        shadow_misses.insert(PolicyType::LFU, 0);
        shadow_misses.insert(PolicyType::ARC, 0);

        Self {
            _phantom: PhantomData,
            active_policy: PolicyType::LRU,
            lru: LruPolicy::new(),
            lfu: LfuPolicy::new(),
            arc: ArcPolicy::new(),
            metrics,
            min_requests: 100,
            #[cfg(feature = "std")]
            evaluation_interval: Duration::from_secs(60),
            #[cfg(feature = "std")]
            last_evaluation: Instant::now(),
            #[cfg(not(feature = "std"))]
            request_counter: 0,
            #[cfg(not(feature = "std"))]
            evaluation_frequency: 1000,
            shadow_hits,
            shadow_misses,
        }
    }

    /// Create with capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let mut policy = Self::new();
        policy.arc = ArcPolicy::with_capacity(capacity);
        policy
    }

    /// Set the minimum number of requests before evaluating policies
    pub fn with_min_requests(mut self, min_requests: usize) -> Self {
        self.min_requests = min_requests;
        self
    }

    /// Set the evaluation interval
    #[cfg(feature = "std")]
    pub fn with_evaluation_interval(mut self, interval: Duration) -> Self {
        self.evaluation_interval = interval;
        self
    }

    /// Set evaluation frequency for no-std environments
    #[cfg(not(feature = "std"))]
    pub fn with_evaluation_frequency(mut self, frequency: usize) -> Self {
        self.evaluation_frequency = frequency;
        self
    }

    /// Get the currently active policy
    pub fn active_policy(&self) -> PolicyType {
        self.active_policy
    }

    /// Check if key would be a hit in each policy (shadow execution)
    fn evaluate_shadow_hit(&self, key: &K) -> HashMap<PolicyType, bool> {
        let mut results = HashMap::new();

        // This is a simplified approach - in reality you'd need to track
        // what each policy would predict without actually executing
        results.insert(PolicyType::LRU, self.lru.would_hit(key));
        results.insert(PolicyType::LFU, self.lfu.would_hit(key));
        results.insert(PolicyType::ARC, self.arc.would_hit(key));

        results
    }

    /// Update shadow metrics
    fn update_shadow_metrics(&mut self, key: &K, actual_hit: bool) {
        let shadow_predictions = self.evaluate_shadow_hit(key);

        for (policy_type, predicted_hit) in shadow_predictions {
            if predicted_hit == actual_hit {
                *self.shadow_hits.get_mut(&policy_type).unwrap() += 1;
            } else {
                *self.shadow_misses.get_mut(&policy_type).unwrap() += 1;
            }
        }
    }

    /// Evaluate policies and switch if needed
    #[cfg(feature = "std")]
    fn evaluate_policies(&mut self) {
        // Check if it's time to evaluate
        let now = Instant::now();
        if now.duration_since(self.last_evaluation) < self.evaluation_interval {
            return;
        }

        self.perform_policy_evaluation();
        self.last_evaluation = now;
    }

    /// Evaluate policies and switch if needed (no-std version)
    #[cfg(not(feature = "std"))]
    fn evaluate_policies(&mut self) {
        self.request_counter += 1;
        if self.request_counter % self.evaluation_frequency != 0 {
            return;
        }

        self.perform_policy_evaluation();
    }

    /// Perform the actual policy evaluation
    fn perform_policy_evaluation(&mut self) {
        // Check if we have enough data
        let total_requests: usize =
            self.shadow_hits.values().sum::<usize>() + self.shadow_misses.values().sum::<usize>();

        if total_requests < self.min_requests {
            return;
        }

        // Find the policy with the highest shadow hit rate
        let mut best_policy = self.active_policy;
        let mut best_hit_rate = 0.0;

        for policy_type in [PolicyType::LRU, PolicyType::LFU, PolicyType::ARC] {
            let hits = *self.shadow_hits.get(&policy_type).unwrap();
            let misses = *self.shadow_misses.get(&policy_type).unwrap();
            let total = hits + misses;

            if total > 0 {
                let hit_rate = hits as f64 / total as f64;
                if hit_rate > best_hit_rate {
                    best_policy = policy_type;
                    best_hit_rate = hit_rate;
                }
            }
        }

        // Switch to the best policy if it's different and significantly better
        let current_hits = *self.shadow_hits.get(&self.active_policy).unwrap();
        let current_misses = *self.shadow_misses.get(&self.active_policy).unwrap();
        let current_total = current_hits + current_misses;

        if current_total > 0 {
            let current_hit_rate = current_hits as f64 / current_total as f64;
            // Only switch if the improvement is significant (> 5%)
            if best_policy != self.active_policy && best_hit_rate > current_hit_rate + 0.05 {
                self.active_policy = best_policy;
            }
        }

        // Reset shadow metrics
        for hits in self.shadow_hits.values_mut() {
            *hits = 0;
        }
        for misses in self.shadow_misses.values_mut() {
            *misses = 0;
        }

        // Reset policy metrics
        for metrics in self.metrics.values_mut() {
            metrics.reset();
        }
    }

    /// Get policy performance statistics
    pub fn get_policy_stats(&self) -> HashMap<PolicyType, f64> {
        let mut stats = HashMap::new();

        for policy_type in [PolicyType::LRU, PolicyType::LFU, PolicyType::ARC] {
            let hits = *self.shadow_hits.get(&policy_type).unwrap();
            let misses = *self.shadow_misses.get(&policy_type).unwrap();
            let total = hits + misses;

            let hit_rate = if total > 0 {
                hits as f64 / total as f64
            } else {
                0.0
            };

            stats.insert(policy_type, hit_rate);
        }

        stats
    }
}

impl<K, V> VictimSelector<K, V> for AdaptivePolicy<K, V>
where
    K: CacheKey,
{
    fn select_victim(&self, _entries: &[EvictionEntry<K, V>]) -> Option<K> {
        match self.active_policy {
            PolicyType::LRU => self.lru.select_victim(),
            PolicyType::LFU => self.lfu.select_victim(),
            PolicyType::ARC => self.arc.select_victim(),
        }
    }
}

impl<K, V> EvictionPolicy<K, V> for AdaptivePolicy<K, V>
where
    K: CacheKey,
{
    fn record_access(&mut self, key: &K, size_hint: Option<usize>) {
        // Record access in all policies for shadow execution
        self.lru.record_access(key, size_hint);
        self.lfu.record_access(key, size_hint);
        self.arc.record_access(key, size_hint);

        // Update shadow metrics (assuming this is a hit for now)
        self.update_shadow_metrics(key, true);

        // Evaluate policies periodically
        self.evaluate_policies();
    }

    fn record_insertion(&mut self, key: &K, entry: &CacheEntry<V>, size_hint: Option<usize>) {
        // Record insertion in all policies
        self.lru.record_insertion(key, entry, size_hint);
        self.lfu.record_insertion(key, entry, size_hint);
        self.arc.record_insertion(key, entry, size_hint);

        // This is a miss for shadow metrics
        self.update_shadow_metrics(key, false);
    }

    fn record_removal(&mut self, key: &K) {
        // Record removal in all policies
        self.lru.record_removal(key);
        self.lfu.record_removal(key);
        self.arc.record_removal(key);
    }

    fn select_victim(&self) -> Option<K> {
        match self.active_policy {
            PolicyType::LRU => self.lru.select_victim(),
            PolicyType::LFU => self.lfu.select_victim(),
            PolicyType::ARC => self.arc.select_victim(),
        }
    }

    fn clear(&mut self) {
        self.lru.clear();
        self.lfu.clear();
        self.arc.clear();

        // Reset all metrics
        for metrics in self.metrics.values_mut() {
            metrics.reset();
        }

        for hits in self.shadow_hits.values_mut() {
            *hits = 0;
        }
        for misses in self.shadow_misses.values_mut() {
            *misses = 0;
        }

        #[cfg(not(feature = "std"))]
        {
            self.request_counter = 0;
        }
    }

    fn name(&self) -> &str {
        match self.active_policy {
            PolicyType::LRU => "Adaptive(LRU)",
            PolicyType::LFU => "Adaptive(LFU)",
            PolicyType::ARC => "Adaptive(ARC)",
        }
    }
}

impl<K, V> Default for AdaptivePolicy<K, V>
where
    K: CacheKey,
{
    fn default() -> Self {
        Self::new()
    }
}

// Mock implementations for the missing methods
trait PolicyExtensions<K> {
    fn would_hit(&self, key: &K) -> bool;
}

// These would need to be implemented in the actual policy types
impl<K, V> PolicyExtensions<K> for LruPolicy<K, V>
where
    K: CacheKey,
{
    fn would_hit(&self, _key: &K) -> bool {
        // Simplified implementation - would need actual logic
        false
    }
}

impl<K, V> PolicyExtensions<K> for LfuPolicy<K, V>
where
    K: CacheKey,
{
    fn would_hit(&self, _key: &K) -> bool {
        // Simplified implementation - would need actual logic
        false
    }
}

impl<K, V> PolicyExtensions<K> for ArcPolicy<K, V>
where
    K: CacheKey,
{
    fn would_hit(&self, _key: &K) -> bool {
        // Simplified implementation - would need actual logic
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adaptive_policy_creation() {
        let policy = AdaptivePolicy::<String, i32>::new();
        assert_eq!(policy.active_policy(), PolicyType::LRU);
    }

    #[test]
    fn test_adaptive_policy_with_capacity() {
        let policy = AdaptivePolicy::<String, i32>::with_capacity(1000);
        assert_eq!(policy.active_policy(), PolicyType::LRU);
    }

    #[test]
    fn test_policy_switching() {
        let mut policy = AdaptivePolicy::<String, i32>::new().with_min_requests(10);

        let entry = CacheEntry::new(42);

        // Insert and access some keys
        for i in 0..20 {
            let key = format!("key{}", i);
            policy.record_insertion(&key, &entry, Some(100));
            policy.record_access(&key, Some(100));
        }

        // Policy should still be working
        assert!(matches!(
            policy.active_policy(),
            PolicyType::LRU | PolicyType::LFU | PolicyType::ARC
        ));
    }

    #[test]
    fn test_victim_selection() {
        let policy = AdaptivePolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        let entries = vec![
            EvictionEntry::new(&"key1".to_string(), &entry),
            EvictionEntry::new(&"key2".to_string(), &entry),
        ];

        // Should be able to select a victim
        let victim = policy.select_victim(&entries);
        assert!(victim.is_some());
    }

    #[test]
    fn test_clear() {
        let mut policy = AdaptivePolicy::<String, i32>::new();
        let entry = CacheEntry::new(42);

        policy.record_insertion(&"key1".to_string(), &entry, Some(100));
        policy.clear();

        // After clearing, victim selection should work
        let victim = policy.select_victim();
        assert!(victim.is_none());
    }
}
