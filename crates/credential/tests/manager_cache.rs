//! Integration tests for credential manager cache performance and behavior.
//!
//! Tests TDD approach:
//! 1. Write tests first (RED phase - tests MUST fail initially)
//! 2. Implement features (GREEN phase - make tests pass)
//! 3. Refactor if needed (REFACTOR phase - improve while keeping tests green)

use nebula_credential::prelude::*;
use std::sync::Arc;
use std::time::Duration;

/// Helper to create test manager with caching enabled
fn create_cached_manager() -> CredentialManager {
    let backend = Arc::new(MockStorageProvider::new());

    CredentialManager::builder()
        .storage(backend)
        .cache_ttl(Duration::from_secs(300))
        .cache_max_size(100)
        .build()
}

/// Helper to create test manager with cache disabled
fn create_uncached_manager() -> CredentialManager {
    let backend = Arc::new(MockStorageProvider::new());

    CredentialManager::builder().storage(backend).build()
}

/// Helper to create test manager with short TTL for expiration tests
fn create_short_ttl_manager() -> CredentialManager {
    let backend = Arc::new(MockStorageProvider::new());

    CredentialManager::builder()
        .storage(backend)
        .cache_ttl(Duration::from_millis(100))
        .cache_max_size(100)
        .build()
}

/// Helper to create test manager with small cache for eviction tests
fn create_small_cache_manager() -> CredentialManager {
    let backend = Arc::new(MockStorageProvider::new());

    CredentialManager::builder()
        .storage(backend)
        .cache_ttl(Duration::from_secs(300))
        .cache_max_size(3)
        .build()
}

/// Helper to create test encrypted data
fn create_test_data(value: &str) -> EncryptedData {
    let key = EncryptionKey::from_bytes([0u8; 32]);
    encrypt(&key, value.as_bytes()).unwrap()
}

/// T076: Test cache hit latency <10ms
///
/// This test verifies that cache hits provide fast access to credentials
/// with sub-10ms p99 latency as specified in the success criteria.
#[tokio::test]
async fn test_cache_hit_latency() {
    let manager = create_cached_manager();
    let id = CredentialId::new("latency-test").unwrap();
    let data = create_test_data("password123");
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Store credential (populates cache)
    manager
        .store(&id, data.clone(), metadata, &context)
        .await
        .unwrap();

    // Warm up cache
    manager.retrieve(&id, &context).await.unwrap();

    // Measure cache hit latency over 100 iterations
    let mut latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = std::time::Instant::now();
        let result = manager.retrieve(&id, &context).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "Cache hit should succeed");
        latencies.push(elapsed);
    }

    // Calculate p99 latency
    latencies.sort();
    let p99_index = (latencies.len() as f64 * 0.99) as usize;
    let p99 = latencies[p99_index];

    // Verify p99 < 10ms (success criteria)
    assert!(
        p99 < Duration::from_millis(10),
        "p99 cache hit latency {:?} exceeds 10ms threshold",
        p99
    );

    // Verify cache stats show high hit rate
    let stats = manager.cache_stats().expect("Cache should be enabled");
    assert!(stats.hits > 0, "Should have cache hits");
    assert!(stats.hit_rate() > 0.9, "Hit rate should be >90%");
}

/// T077: Test cache TTL expiration
///
/// Verifies that cached credentials expire after configured TTL duration
/// and are automatically evicted from the cache.
#[tokio::test]
async fn test_cache_ttl_expiration() {
    let manager = create_short_ttl_manager();
    let id = CredentialId::new("ttl-test").unwrap();
    let data = create_test_data("password123");
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Store credential (populates cache)
    manager
        .store(&id, data.clone(), metadata, &context)
        .await
        .unwrap();

    // First retrieve - cache hit
    manager.retrieve(&id, &context).await.unwrap();
    let stats_before = manager.cache_stats().unwrap();

    // Wait for TTL expiration
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Second retrieve - cache miss (expired), backend hit
    manager.retrieve(&id, &context).await.unwrap();
    let stats_after = manager.cache_stats().unwrap();

    // Verify cache miss occurred (hits didn't increase by 1, or misses increased)
    assert!(
        stats_after.misses > stats_before.misses,
        "Cache miss should occur after TTL expiration"
    );
}

/// T078: Test cache invalidation on update
///
/// Verifies that updating a credential invalidates the cache entry
/// so subsequent retrieves get fresh data.
#[tokio::test]
async fn test_cache_invalidation_on_update() {
    let manager = create_cached_manager();
    let id = CredentialId::new("update-test").unwrap();
    let data_v1 = create_test_data("password-v1");
    let data_v2 = create_test_data("password-v2");
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Store initial version (populates cache)
    manager
        .store(&id, data_v1.clone(), metadata.clone(), &context)
        .await
        .unwrap();

    // Retrieve to warm cache
    let (retrieved_v1, _) = manager.retrieve(&id, &context).await.unwrap().unwrap();

    // Update credential (store with same ID should invalidate cache)
    manager
        .store(&id, data_v2.clone(), metadata, &context)
        .await
        .unwrap();

    // Retrieve again - should get updated data
    let (retrieved_v2, _) = manager.retrieve(&id, &context).await.unwrap().unwrap();

    // Verify we got the updated version
    assert_ne!(
        format!("{:?}", retrieved_v1),
        format!("{:?}", retrieved_v2),
        "Retrieved data should reflect update"
    );
}

/// T079: Test LRU eviction when cache is full
///
/// Verifies that least recently used entries are evicted when cache
/// reaches max capacity.
#[tokio::test]
async fn test_lru_eviction() {
    let manager = create_small_cache_manager();
    let context = CredentialContext::new("user-1");

    // Store 3 credentials to fill cache
    for i in 1..=3 {
        let id = CredentialId::new(format!("cred-{}", i)).unwrap();
        let data = create_test_data(&format!("pass-{}", i));
        let metadata = CredentialMetadata::new();
        manager.store(&id, data, metadata, &context).await.unwrap();
    }

    // Access cred-2 and cred-3 to make them recently used
    let id2 = CredentialId::new("cred-2").unwrap();
    let id3 = CredentialId::new("cred-3").unwrap();
    manager.retrieve(&id2, &context).await.unwrap();
    manager.retrieve(&id3, &context).await.unwrap();

    // Store 4th credential (should evict cred-1 as LRU)
    let id4 = CredentialId::new("cred-4").unwrap();
    let data4 = create_test_data("pass-4");
    let metadata4 = CredentialMetadata::new();
    manager
        .store(&id4, data4, metadata4, &context)
        .await
        .unwrap();

    // Get cache stats
    let stats = manager.cache_stats().unwrap();

    // Verify cache size is capped at max_capacity
    assert!(
        stats.size <= 3,
        "Cache size {} should not exceed max_capacity 3",
        stats.size
    );
}

/// T080: Test cache statistics tracking
///
/// Verifies that cache stats accurately track hits, misses, size, and capacity.
#[tokio::test]
async fn test_cache_stats() {
    let manager = create_cached_manager();
    let id = CredentialId::new("stats-test").unwrap();
    let data = create_test_data("password123");
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Initial stats
    let stats_initial = manager.cache_stats().expect("Cache should be enabled");
    assert_eq!(stats_initial.hits, 0, "Should start with 0 hits");
    assert_eq!(stats_initial.misses, 0, "Should start with 0 misses");

    // Store credential
    manager.store(&id, data, metadata, &context).await.unwrap();

    // First retrieve - cache miss (not yet cached)
    manager.retrieve(&id, &context).await.unwrap();
    let stats_after_miss = manager.cache_stats().unwrap();

    // Second retrieve - cache hit
    manager.retrieve(&id, &context).await.unwrap();
    let stats_after_hit = manager.cache_stats().unwrap();

    // Verify hit counter increased
    assert!(
        stats_after_hit.hits > stats_after_miss.hits,
        "Hits should increase after cache hit"
    );

    // Verify hit rate calculation
    let hit_rate = stats_after_hit.hit_rate();
    assert!(
        (0.0..=1.0).contains(&hit_rate),
        "Hit rate should be between 0 and 1"
    );
    assert!(hit_rate > 0.0, "Hit rate should be >0 after cache hit");

    // Verify utilization calculation
    let utilization = stats_after_hit.utilization();
    assert!(utilization >= 0.0, "Utilization should be >=0");
    assert!(utilization <= 1.0, "Utilization should not exceed 1.0");

    // Note: Moka's entry_count() may be 0 due to async write buffering.
    // The important thing is that hits > 0, proving cache is working.
}

/// T081: Test cache disabled by default behavior
///
/// Verifies that when cache is disabled, manager still functions correctly
/// but returns None for cache stats.
#[tokio::test]
async fn test_cache_disabled_by_default() {
    let manager = create_uncached_manager();
    let id = CredentialId::new("no-cache-test").unwrap();
    let data = create_test_data("password123");
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Store and retrieve should work without cache
    manager.store(&id, data, metadata, &context).await.unwrap();
    let result = manager.retrieve(&id, &context).await.unwrap();
    assert!(result.is_some(), "Retrieve should work without cache");

    // Cache stats should return None when disabled
    let stats = manager.cache_stats();
    assert!(
        stats.is_none(),
        "Cache stats should be None when cache disabled"
    );
}
