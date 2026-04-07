//! Concurrent cache with TTL expiry and built-in metrics
//!
//! Demonstrates `ConcurrentComputeCache` — the primary cache type for
//! multi-threaded workloads like expression/template parsing.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nebula_memory::cache::{CacheConfig, ConcurrentComputeCache};

fn main() {
    // === Example 1: Basic get-or-compute ===
    println!("=== 1. Basic get-or-compute ===");

    let cache = ConcurrentComputeCache::<String, i64>::new(100);

    // First call computes the value
    let result = cache.get_or_compute("answer".into(), || Ok(42)).unwrap();
    println!("Computed: {result}");

    // Second call returns cached value (compute_fn is never called)
    let result = cache
        .get_or_compute("answer".into(), || panic!("should not be called"))
        .unwrap();
    println!("Cached:   {result}");

    let stats = cache.stats();
    println!("Hits: {}, Misses: {}", stats.hits, stats.misses);

    // === Example 2: TTL expiry ===
    println!("\n=== 2. TTL expiry ===");

    let config = CacheConfig::new(100).with_ttl(Duration::from_millis(200));
    let cache = ConcurrentComputeCache::<String, String>::with_config(config);

    cache.insert("greeting".into(), "hello".into()).unwrap();
    println!("Before TTL: {:?}", cache.get(&"greeting".into()));

    thread::sleep(Duration::from_millis(300));
    println!("After TTL:  {:?}", cache.get(&"greeting".into()));

    // === Example 3: High-throughput preset ===
    println!("\n=== 3. High-throughput config preset ===");

    let config = CacheConfig::for_high_throughput(1_000);
    let cache = ConcurrentComputeCache::<u64, Vec<u8>>::with_config(config);

    for i in 0..500 {
        cache.get_or_compute(i, || Ok(vec![0u8; 64])).unwrap();
    }

    let stats = cache.stats();
    println!(
        "Entries: {}, Insertions: {}, Hit rate: {:.1}%",
        cache.len(),
        stats.insertions,
        stats.hit_rate()
    );

    // === Example 4: Concurrent access ===
    println!("\n=== 4. Concurrent access (10 threads) ===");

    let cache = Arc::new(ConcurrentComputeCache::<String, usize>::new(100));
    let mut handles = vec![];

    for thread_id in 0..10 {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let key = format!("key_{}", i % 20);
                c.get_or_compute(key, || Ok(thread_id * 1000 + i)).unwrap();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let stats = cache.stats();
    println!(
        "Total ops: {}, Hits: {}, Misses: {}, Hit rate: {:.1}%",
        stats.total_requests(),
        stats.hits,
        stats.misses,
        stats.hit_rate()
    );

    // === Example 5: Stats and reset ===
    println!("\n=== 5. Stats snapshot and reset ===");
    println!("Before reset: {} hits", cache.stats().hits);

    cache.reset_stats();
    println!("After reset:  {} hits", cache.stats().hits);

    // New operations after reset
    cache.get(&"key_0".into());
    cache.get(&"nonexistent".into());
    let s = cache.stats();
    println!("After 2 gets: hits={}, misses={}", s.hits, s.misses);
}
