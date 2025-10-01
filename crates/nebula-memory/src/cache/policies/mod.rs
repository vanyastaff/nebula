//! Cache eviction policies
//!
//! This module provides various cache eviction policies that determine
//! which entries to remove when a cache is full.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::HashMap,
    hash::Hash,
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, vec::Vec},
    core::hash::Hash,
    core::time::Duration,
    hashbrown::HashMap,
};

use super::compute::{CacheEntry, CacheKey};
use crate::core::error::MemoryResult;

// Re-export policy modules
mod adaptive;
mod arc;
mod lfu;
mod lru;
mod ttl;

pub use adaptive::AdaptivePolicy;
pub use arc::ArcPolicy;
pub use lfu::LfuPolicy;
pub use lru::LruPolicy;
pub use ttl::TtlPolicy;

/// Type-erased entry for eviction policy
pub type EvictionEntry<'a, K, V> = (&'a K, &'a CacheEntry<V>);

/// Trait for selecting victims (can be used with dyn)
pub trait VictimSelector<K, V>: Send + Sync {
    /// Choose a key to evict from given entries
    fn select_victim<'a>(&self, entries: &[EvictionEntry<'a, K, V>]) -> Option<K>;
}

/// Trait for cache eviction policies
pub trait EvictionPolicy<K, V>: Send + Sync {
    /// Record an access to a key
    fn record_access(&mut self, key: &K);

    /// Record an insertion of a key
    fn record_insertion(&mut self, key: &K, entry: &CacheEntry<V>);

    /// Record a removal of a key
    fn record_removal(&mut self, key: &K);

    /// Convert to a victim selector
    fn as_victim_selector(&self) -> &dyn VictimSelector<K, V>;

    /// Get the name of this policy
    fn name(&self) -> &str;
}

/// Helper method to choose a victim using original iterator-based approach
pub fn choose_victim<'a, K, V, I>(
    selector: &dyn VictimSelector<K, V>,
    entries: I,
) -> Option<K>
where
    I: Iterator<Item = (&'a K, &'a CacheEntry<V>)>,
    K: 'a + Clone,
    V: 'a,
{
    // Collect entries to a Vec for selector
    let entries: Vec<EvictionEntry<K, V>> = entries.collect();
    selector.select_victim(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_names() {
        // Test direct policy creation and name verification
        let lru: Box<dyn EvictionPolicy<String, usize>> = Box::new(LruPolicy::<String, usize>::new());
        assert_eq!(lru.name(), "LRU");

        let lfu: Box<dyn EvictionPolicy<String, usize>> = Box::new(LfuPolicy::<String, usize>::new());
        assert_eq!(lfu.name(), "LFU");

        let ttl: Box<dyn EvictionPolicy<String, usize>> = Box::new(TtlPolicy::<String>::new(std::time::Duration::from_secs(60)));
        assert_eq!(ttl.name(), "TTL");

        let arc: Box<dyn EvictionPolicy<String, usize>> = Box::new(ArcPolicy::<String, usize>::new(100));
        assert_eq!(arc.name(), "ARC");

        let adaptive: Box<dyn EvictionPolicy<String, usize>> = Box::new(AdaptivePolicy::<String, usize>::new(100));
        assert_eq!(adaptive.name(), "Adaptive");
    }
}
