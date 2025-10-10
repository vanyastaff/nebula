#![cfg_attr(not(feature = "std"), no_std)]
pub mod fifo;
// TODO: Fix lfu module after migration
// pub mod lfu;
pub mod lru;
pub mod random;
pub mod ttl;

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, vec::Vec},
    core::hash::Hash,
    core::time::Duration,
    hashbrown::HashMap,
};

use super::CacheEntry;

// Re-export policy modules
pub use fifo::FifoPolicy;
// pub use lfu::LfuPolicy;
pub use lru::LruPolicy;
pub use random::RandomPolicy;
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
pub fn choose_victim<'a, K, V, I>(selector: &dyn VictimSelector<K, V>, entries: I) -> Option<K>
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
        // Test that remaining policies have correct names
        let lru = LruPolicy::<String, usize>::new();
        assert_eq!(lru.name(), "LRU");

        let fifo = FifoPolicy::<String, usize>::new();
        assert_eq!(fifo.name(), "FIFO");

        let random = RandomPolicy::<String, usize>::new();
        assert_eq!(random.name(), "Random");

        #[cfg(feature = "std")]
        {
            let ttl = TtlPolicy::<String>::new(std::time::Duration::from_secs(60));
            assert_eq!(ttl.name(), "TTL");
        }
    }
}
