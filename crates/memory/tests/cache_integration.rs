//! Integration tests for `ConcurrentComputeCache`
//!
//! Validates cross-module behavior: config presets + cache + stats + TTL.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nebula_memory::cache::{CacheConfig, CacheStats, ConcurrentComputeCache, StatsProvider};

// ---------------------------------------------------------------------------
// Config → Cache → Stats end-to-end
// ---------------------------------------------------------------------------

#[test]
fn high_throughput_preset_populates_stats() {
    let config = CacheConfig::for_high_throughput(500);
    let cache = ConcurrentComputeCache::<u64, u64>::with_config(config);

    for i in 0..200 {
        cache.get_or_compute(i, || Ok(i * 2)).unwrap();
    }
    // Re-read half — should all be hits
    for i in 0..100 {
        cache.get_or_compute(i, || panic!("should hit")).unwrap();
    }

    let s = cache.stats();
    assert_eq!(s.misses, 200);
    assert_eq!(s.hits, 100);
    assert!(s.insertions >= 200);
}

#[test]
fn config_validation_then_use() {
    let config = CacheConfig::new(50)
        .with_ttl(Duration::from_secs(60))
        .with_initial_capacity(25);

    assert!(config.validate().is_ok());

    let cache = ConcurrentComputeCache::<String, i32>::with_config(config);
    cache.insert("k".into(), 1).unwrap();
    assert_eq!(cache.get(&"k".into()), Some(1));
}

// ---------------------------------------------------------------------------
// Eviction under pressure
// ---------------------------------------------------------------------------

#[test]
fn eviction_pressure_never_exceeds_capacity() {
    let cache = ConcurrentComputeCache::<u64, Vec<u8>>::new(10);

    for i in 0..1_000 {
        cache.get_or_compute(i, || Ok(vec![0u8; 32])).unwrap();
        assert!(
            cache.len() <= 10,
            "len={} after insert {i}, should be <= 10",
            cache.len()
        );
    }

    let s = cache.stats();
    assert!(
        s.evictions >= 990,
        "expected ~990 evictions, got {}",
        s.evictions
    );
}

#[test]
fn direct_insert_respects_capacity() {
    let cache = ConcurrentComputeCache::<u32, u32>::new(5);

    for i in 0..50 {
        cache.insert(i, i * 10).unwrap();
        assert!(cache.len() <= 5);
    }
}

// ---------------------------------------------------------------------------
// TTL
// ---------------------------------------------------------------------------

#[test]
fn ttl_entries_expire_on_read() {
    let config = CacheConfig::new(100).with_ttl(Duration::from_millis(50));
    let cache = ConcurrentComputeCache::<String, i32>::with_config(config);

    cache.insert("a".into(), 1).unwrap();
    cache.insert("b".into(), 2).unwrap();

    assert_eq!(cache.get(&"a".into()), Some(1));

    thread::sleep(Duration::from_millis(100));

    assert_eq!(cache.get(&"a".into()), None, "a should have expired");
    assert_eq!(cache.get(&"b".into()), None, "b should have expired");
}

#[test]
fn ttl_get_or_compute_recomputes_expired() {
    let config = CacheConfig::new(100).with_ttl(Duration::from_millis(50));
    let cache = ConcurrentComputeCache::<String, u32>::with_config(config);

    cache.get_or_compute("k".into(), || Ok(1)).unwrap();
    thread::sleep(Duration::from_millis(100));

    let v = cache.get_or_compute("k".into(), || Ok(2)).unwrap();
    assert_eq!(v, 2, "should recompute after expiry");
}

#[test]
fn no_ttl_never_expires() {
    let cache = ConcurrentComputeCache::<String, i32>::new(100);

    cache.insert("k".into(), 42).unwrap();
    thread::sleep(Duration::from_millis(100));

    assert_eq!(cache.get(&"k".into()), Some(42));
}

// ---------------------------------------------------------------------------
// Concurrent access
// ---------------------------------------------------------------------------

#[test]
fn concurrent_metrics_sum_to_total_ops() {
    let cache = Arc::new(ConcurrentComputeCache::<String, usize>::new(500));
    let threads = 20;
    let ops_per_thread = 500;
    let mut handles = vec![];

    for t in 0..threads {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..ops_per_thread {
                let key = format!("key_{}", i % 50);
                c.get_or_compute(key, || Ok(t * 1000 + i)).unwrap();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let s = cache.stats();
    let total = threads * ops_per_thread;
    assert_eq!(
        s.hits + s.misses,
        total as u64,
        "hits({}) + misses({}) should equal {total}",
        s.hits,
        s.misses
    );
    assert!(s.hits > 0, "should have some cache hits");
    assert!(s.insertions <= 50, "at most 50 unique keys");
}

#[test]
fn concurrent_ttl_expiry() {
    let config = CacheConfig::new(1000).with_ttl(Duration::from_millis(100));
    let cache = Arc::new(ConcurrentComputeCache::<u64, u64>::with_config(config));

    // Phase 1: populate
    for i in 0..100 {
        cache.insert(i, i).unwrap();
    }
    assert_eq!(cache.len(), 100);

    // Wait for TTL
    thread::sleep(Duration::from_millis(200));

    // Phase 2: concurrent reads on expired entries
    let mut handles = vec![];
    for _ in 0..10 {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            let mut expired = 0u64;
            for i in 0..100 {
                if c.get(&i).is_none() {
                    expired += 1;
                }
            }
            expired
        }));
    }

    let total_expired: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
    // All entries should be expired for all threads
    assert_eq!(total_expired, 1000, "all reads should see expired entries");
}

// ---------------------------------------------------------------------------
// StatsProvider trait
// ---------------------------------------------------------------------------

#[test]
fn stats_provider_trait_works() {
    let cache = ConcurrentComputeCache::<String, i32>::new(10);
    cache.insert("x".into(), 1).unwrap();
    cache.get(&"x".into());

    // Use via trait object
    let provider: &dyn StatsProvider = &cache;
    let s: CacheStats = provider.stats();
    assert!(s.hits >= 1);

    provider.reset_stats();
    assert_eq!(provider.stats().hits, 0);
}

// ---------------------------------------------------------------------------
// Clone shares state
// ---------------------------------------------------------------------------

#[test]
fn clone_shares_entries_and_stats() {
    let cache = ConcurrentComputeCache::<String, i32>::new(10);
    let clone = cache.clone();

    cache.insert("k".into(), 42).unwrap();
    assert_eq!(clone.get(&"k".into()), Some(42), "clone should see insert");

    // Stats are shared too
    assert!(clone.stats().hits >= 1);
}
