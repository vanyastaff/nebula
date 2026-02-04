//! Integration tests for CredentialManager builder pattern.
//!
//! These tests verify:
//! - Fluent API usability
//! - Compile-time type safety (storage required)
//! - Method chaining
//! - Configuration options

use nebula_credential::prelude::*;
use std::sync::Arc;
use std::time::Duration;

/// T098: Test builder fluent API
///
/// Verifies that the builder provides a fluent, chainable API
/// for configuring the credential manager.
#[tokio::test]
async fn test_builder_fluent_api() {
    let storage = Arc::new(MockStorageProvider::new());

    // Test fluent chaining
    let manager = CredentialManager::builder()
        .storage(storage)
        .cache_ttl(Duration::from_secs(300))
        .cache_max_size(1000)
        .build();

    // Verify manager was created successfully by using it
    let id = CredentialId::new("test-cred").unwrap();
    let key = EncryptionKey::from_bytes([0u8; 32]);
    let data = encrypt(&key, b"test-data").unwrap();
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Store and retrieve to verify manager works
    manager.store(&id, data, metadata, &context).await.unwrap();
    let result = manager.retrieve(&id, &context).await.unwrap();
    assert!(
        result.is_some(),
        "Manager should work after builder construction"
    );
}

/// T099: Test builder cache configuration
///
/// Verifies that cache configuration methods properly enable
/// and configure the cache layer.
#[tokio::test]
async fn test_builder_cache_config() {
    let storage = Arc::new(MockStorageProvider::new());

    // Build manager with cache configuration
    let manager = CredentialManager::builder()
        .storage(storage)
        .cache_ttl(Duration::from_secs(60))
        .cache_max_size(50)
        .build();

    // Verify cache is enabled
    let stats = manager.cache_stats();
    assert!(
        stats.is_some(),
        "Cache should be enabled after configuration"
    );

    let stats = stats.unwrap();
    assert_eq!(
        stats.max_capacity, 50,
        "Cache max capacity should match configuration"
    );
}

/// T100: Test builder with multiple configuration options
///
/// Verifies that multiple configuration methods can be chained
/// and all work correctly together.
#[tokio::test]
async fn test_builder_multiple_configs() {
    let storage = Arc::new(MockStorageProvider::new());

    // Build manager with multiple configurations
    let manager = CredentialManager::builder()
        .storage(storage)
        .cache_ttl(Duration::from_secs(600))
        .cache_max_size(500)
        .build();

    // Verify cache is enabled with correct settings
    let stats = manager.cache_stats().expect("Cache should be enabled");
    assert_eq!(
        stats.max_capacity, 500,
        "Cache capacity should match configuration"
    );

    // Verify manager works
    let id = CredentialId::new("test").unwrap();
    let key = EncryptionKey::from_bytes([0u8; 32]);
    let data = encrypt(&key, b"data").unwrap();
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    manager.store(&id, data, metadata, &context).await.unwrap();
}

/// T101: Test builder default values
///
/// Verifies that the builder uses sensible defaults when
/// optional configuration methods are not called.
#[tokio::test]
async fn test_builder_default_values() {
    let storage = Arc::new(MockStorageProvider::new());

    // Build manager with minimal configuration (only storage)
    let manager = CredentialManager::builder().storage(storage).build();

    // Verify cache is disabled by default
    let stats = manager.cache_stats();
    assert!(stats.is_none(), "Cache should be disabled by default");

    // Verify manager still works
    let id = CredentialId::new("test").unwrap();
    let key = EncryptionKey::from_bytes([0u8; 32]);
    let data = encrypt(&key, b"data").unwrap();
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    manager.store(&id, data, metadata, &context).await.unwrap();
}

/// T097: Test that builder requires storage (compile-time check)
///
/// This test verifies compile-time type safety by showing that
/// the builder works when storage is provided.
///
/// Note: The compile_fail test for missing storage is in the rustdoc
/// comments in src/manager/manager.rs.
#[tokio::test]
async fn test_builder_enforces_required_storage() {
    // This test verifies that we CAN build when storage is provided
    let storage = Arc::new(MockStorageProvider::new());
    let _manager = CredentialManager::builder().storage(storage).build();

    // The compile_fail test for missing storage is in the rustdoc:
    // ```compile_fail
    // let manager = CredentialManager::builder().build(); // ERROR: storage required
    // ```
}

/// Additional test: Verify cache_config() method
///
/// Tests the cache_config() method that accepts a full CacheConfig struct.
#[tokio::test]
async fn test_builder_cache_config_struct() {
    let storage = Arc::new(MockStorageProvider::new());

    let cache_config = CacheConfig {
        enabled: true,
        ttl: Some(Duration::from_secs(120)),
        idle_timeout: Some(Duration::from_secs(60)),
        max_capacity: 200,
        eviction_strategy: EvictionStrategy::Lru,
    };

    let manager = CredentialManager::builder()
        .storage(storage)
        .cache_config(cache_config)
        .build();

    // Verify cache is enabled with correct capacity
    let stats = manager.cache_stats().expect("Cache should be enabled");
    assert_eq!(
        stats.max_capacity, 200,
        "Cache capacity should match config"
    );
}
