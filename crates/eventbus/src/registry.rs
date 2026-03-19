//! Multi-bus registry for per-scope isolation (e.g. per-tenant).

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::BackPressurePolicy;
use crate::EventBus;

/// Aggregated snapshot across all buses in an [`EventBusRegistry`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventBusRegistryStats {
    /// Number of active bus instances in the registry.
    pub bus_count: usize,
    /// Total sent events across all buses.
    pub sent_count: u64,
    /// Total dropped events across all buses.
    pub dropped_count: u64,
    /// Total active subscribers across all buses.
    pub subscriber_count: usize,
}

/// Registry that manages multiple isolated [`EventBus`] instances by key.
///
/// Typical usage is per-tenant or per-project isolation, where each key maps to
/// its own independent bus with separate buffering and stats.
///
/// # Concurrency
///
/// - **Double-checked locking:** `get_or_create` uses a read lock first for performance,
///   then upgrades to a write lock only if the bus does not exist. This minimizes contention.
///
/// - **Best-effort pruning:** `prune_without_subscribers` is a best-effort operation.
///   A subscriber created between the check and the prune will survive, which is safe
///   (the bus remains in the registry). The method is advisory and should not be relied
///   upon for guaranteed cleanup.
#[derive(Debug)]
pub struct EventBusRegistry<K, E> {
    buses: RwLock<HashMap<K, Arc<EventBus<E>>>>,
    buffer_size: usize,
    policy: BackPressurePolicy,
}

impl<K, E> EventBusRegistry<K, E>
where
    K: Eq + Hash + Clone,
    E: Clone + Send,
{
    /// Creates a registry with default [`BackPressurePolicy::DropOldest`].
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        Self::with_policy(buffer_size, BackPressurePolicy::default())
    }

    /// Creates a registry with explicit back-pressure policy for each bus.
    #[must_use]
    pub fn with_policy(buffer_size: usize, policy: BackPressurePolicy) -> Self {
        assert!(buffer_size > 0, "EventBusRegistry buffer_size must be > 0");
        Self {
            buses: RwLock::new(HashMap::new()),
            buffer_size,
            policy,
        }
    }

    /// Returns an existing bus for `key` or creates one lazily.
    pub fn get_or_create(&self, key: K) -> Arc<EventBus<E>> {
        if let Some(existing) = self.buses.read().get(&key).cloned() {
            return existing;
        }

        let mut guard = self.buses.write();
        guard
            .entry(key)
            .or_insert_with(|| {
                Arc::new(EventBus::with_policy(self.buffer_size, self.policy.clone()))
            })
            .clone()
    }

    /// Gets a bus by `key` if present.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<Arc<EventBus<E>>> {
        self.buses.read().get(key).cloned()
    }

    /// Removes and returns a bus by `key` if present.
    pub fn remove(&self, key: &K) -> Option<Arc<EventBus<E>>> {
        self.buses.write().remove(key)
    }

    /// Number of active buses in the registry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buses.read().len()
    }

    /// Returns `true` if the registry has no buses.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drops all buses from the registry.
    pub fn clear(&self) {
        self.buses.write().clear();
    }

    /// Removes buses that have no active subscribers.
    ///
    /// Returns number of removed buses.
    pub fn prune_without_subscribers(&self) -> usize {
        let mut guard = self.buses.write();
        let before = guard.len();
        guard.retain(|_, bus| bus.has_subscribers());
        before.saturating_sub(guard.len())
    }

    /// Aggregated snapshot across all buses.
    #[must_use]
    pub fn stats(&self) -> EventBusRegistryStats {
        let guard = self.buses.read();

        let mut snapshot = EventBusRegistryStats {
            bus_count: guard.len(),
            ..EventBusRegistryStats::default()
        };

        for bus in guard.values() {
            let stats = bus.stats();
            snapshot.sent_count = snapshot.sent_count.saturating_add(stats.sent_count);
            snapshot.dropped_count = snapshot.dropped_count.saturating_add(stats.dropped_count);
            snapshot.subscriber_count = snapshot
                .subscriber_count
                .saturating_add(stats.subscriber_count);
        }

        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Event(&'static str);

    #[test]
    fn get_or_create_reuses_existing_bus() {
        let registry = EventBusRegistry::<String, Event>::new(16);

        let a1 = registry.get_or_create("tenant-a".to_string());
        let a2 = registry.get_or_create("tenant-a".to_string());

        assert!(Arc::ptr_eq(&a1, &a2));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn buses_are_isolated_per_key() {
        let registry = EventBusRegistry::<String, Event>::new(16);

        let a_bus = registry.get_or_create("tenant-a".to_string());
        let b_bus = registry.get_or_create("tenant-b".to_string());

        let mut a_sub = a_bus.subscribe();
        let mut b_sub = b_bus.subscribe();

        let _ = a_bus.emit(Event("a"));
        let _ = b_bus.emit(Event("b"));

        assert_eq!(a_sub.try_recv(), Some(Event("a")));
        assert_eq!(b_sub.try_recv(), Some(Event("b")));
    }

    #[test]
    fn prune_without_subscribers_removes_inactive_buses() {
        let registry = EventBusRegistry::<String, Event>::new(16);

        let a = registry.get_or_create("tenant-a".to_string());
        let b = registry.get_or_create("tenant-b".to_string());

        let sub = a.subscribe();
        assert_eq!(registry.prune_without_subscribers(), 1);
        assert_eq!(registry.len(), 1);

        drop(sub);
        let _ = b;
        assert_eq!(registry.prune_without_subscribers(), 1);
        assert!(registry.is_empty());
    }

    #[test]
    fn stats_aggregate_all_buses() {
        let registry = EventBusRegistry::<String, Event>::new(16);

        let a_bus = registry.get_or_create("tenant-a".to_string());
        let b_bus = registry.get_or_create("tenant-b".to_string());

        let _a_sub = a_bus.subscribe();
        let _ = a_bus.emit(Event("a1"));
        let _ = a_bus.emit(Event("a2"));

        let _ = b_bus.emit(Event("b0")); // dropped (no subscribers)

        let stats = registry.stats();
        assert_eq!(stats.bus_count, 2);
        assert_eq!(stats.sent_count, 2);
        assert_eq!(stats.dropped_count, 1);
        assert_eq!(stats.subscriber_count, 1);
    }
}
