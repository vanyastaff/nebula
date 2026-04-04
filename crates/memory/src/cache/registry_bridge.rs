//! Bridge between [`CacheStats`] and the telemetry registry.
//!
//! Call [`sync_to_registry`] periodically to push cache stats
//! into the shared metrics registry for Prometheus export.

use nebula_metrics::naming::{
    NEBULA_CACHE_EVICTIONS, NEBULA_CACHE_HITS, NEBULA_CACHE_MISSES, NEBULA_CACHE_SIZE,
};
use nebula_telemetry::MetricsRegistry;

use super::stats::CacheStats;

/// Pushes a [`CacheStats`] snapshot into the registry as gauge values.
///
/// This is a point-in-time sync — call it periodically (e.g., every 30s)
/// from a background task, not on every cache operation.
///
/// # Examples
///
/// ```ignore
/// use nebula_memory::cache::stats::CacheStats;
/// use nebula_memory::cache::registry_bridge::sync_to_registry;
/// use nebula_telemetry::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let stats = CacheStats { hits: 10, misses: 2, ..Default::default() };
/// sync_to_registry(&stats, &registry);
/// ```
pub fn sync_to_registry(stats: &CacheStats, registry: &MetricsRegistry) {
    registry.gauge(NEBULA_CACHE_HITS).set(stats.hits as i64);
    registry.gauge(NEBULA_CACHE_MISSES).set(stats.misses as i64);
    registry
        .gauge(NEBULA_CACHE_EVICTIONS)
        .set(stats.evictions as i64);
    registry
        .gauge(NEBULA_CACHE_SIZE)
        .set(stats.entry_count as i64);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_pushes_stats_to_registry() {
        let registry = MetricsRegistry::new();
        let stats = CacheStats {
            hits: 42,
            misses: 8,
            evictions: 3,
            insertions: 50,
            deletions: 5,
            entry_count: 45,
            size_bytes: 1024,
        };

        sync_to_registry(&stats, &registry);

        assert_eq!(registry.gauge(NEBULA_CACHE_HITS).get(), 42);
        assert_eq!(registry.gauge(NEBULA_CACHE_MISSES).get(), 8);
        assert_eq!(registry.gauge(NEBULA_CACHE_EVICTIONS).get(), 3);
        assert_eq!(registry.gauge(NEBULA_CACHE_SIZE).get(), 45);
    }

    #[test]
    fn sync_overwrites_previous_values() {
        let registry = MetricsRegistry::new();

        let first = CacheStats {
            hits: 10,
            misses: 5,
            evictions: 1,
            entry_count: 20,
            ..Default::default()
        };
        sync_to_registry(&first, &registry);

        assert_eq!(registry.gauge(NEBULA_CACHE_HITS).get(), 10);

        let second = CacheStats {
            hits: 100,
            misses: 50,
            evictions: 10,
            entry_count: 200,
            ..Default::default()
        };
        sync_to_registry(&second, &registry);

        assert_eq!(registry.gauge(NEBULA_CACHE_HITS).get(), 100);
        assert_eq!(registry.gauge(NEBULA_CACHE_MISSES).get(), 50);
        assert_eq!(registry.gauge(NEBULA_CACHE_EVICTIONS).get(), 10);
        assert_eq!(registry.gauge(NEBULA_CACHE_SIZE).get(), 200);
    }
}
