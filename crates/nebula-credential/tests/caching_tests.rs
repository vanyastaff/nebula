//! Integration tests for token caching behavior

use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use std::sync::Arc;

async fn create_test_manager() -> CredentialManager {
    let store = Arc::new(MockStateStore::new());
    let lock = MockLock::new();
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    CredentialManager::builder()
        .with_store(store)
        .with_lock(lock)
        .with_cache(cache)
        .with_registry(registry)
        .build()
        .expect("Failed to build test manager")
}

#[tokio::test]
async fn test_cache_initialization() {
    let _manager = create_test_manager().await;
    // Manager with cache should be created successfully
}

#[tokio::test]
async fn test_cache_hit_path() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let cache = Arc::new(MockTokenCache::new());
    let store = Arc::new(MockStateStore::new());
    let lock = MockLock::new();
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store)
        .with_lock(lock)
        .with_cache(Arc::clone(&cache) as Arc<dyn nebula_credential::traits::TokenCache>)
        .with_registry(registry)
        .build()
        .unwrap();

    // Create credential
    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    // First call - should cache the token
    let token1 = manager.get_token(&cred_id).await.unwrap();
    let stats_before = cache.stats();

    // Second call - should hit cache (no refresh)
    let token2 = manager.get_token(&cred_id).await.unwrap();
    let stats_after = cache.stats();

    // Should have one more cache hit
    assert_eq!(stats_after.hits, stats_before.hits + 1);
    assert_eq!(token1.token.expose(), token2.token.expose());
}

#[tokio::test]
async fn test_cache_miss_triggers_refresh() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Create credential
    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    // Clear cache to force miss
    manager.cache().unwrap().clear().await.unwrap();

    // This should trigger a refresh from storage
    let token = manager.get_token(&cred_id).await.unwrap();
    assert!(!token.is_expired());
}

#[tokio::test]
async fn test_cache_ttl_expiration() {
    use nebula_credential::core::AccessToken;
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;
    use std::time::{Duration, SystemTime};

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    // Manually put a token with very short TTL
    let cache = manager.cache().unwrap();
    let short_lived_token = AccessToken {
        token: nebula_credential::core::SecureString::new("short-lived"),
        token_type: nebula_credential::core::TokenType::Bearer,
        issued_at: SystemTime::now(),
        expires_at: Some(SystemTime::now() + Duration::from_millis(50)),
        scopes: None,
        claims: Default::default(),
    };

    cache
        .put(cred_id.as_str(), &short_lived_token, Duration::from_millis(50))
        .await
        .unwrap();

    // Wait for TTL to expire
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Should get a fresh token (cache expired)
    let token = manager.get_token(&cred_id).await.unwrap();
    assert_ne!(token.token.expose(), "short-lived");
}

#[tokio::test]
async fn test_cache_invalidation_on_refresh() {
    use nebula_credential::core::AccessToken;
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;
    use std::time::{Duration, SystemTime};

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    // Put an expired token in cache
    let cache = manager.cache().unwrap();
    let expired_token = AccessToken {
        token: nebula_credential::core::SecureString::new("expired-token"),
        token_type: nebula_credential::core::TokenType::Bearer,
        issued_at: SystemTime::now() - Duration::from_secs(7200),
        expires_at: Some(SystemTime::now() - Duration::from_secs(1)),
        scopes: None,
        claims: Default::default(),
    };

    cache
        .put(cred_id.as_str(), &expired_token, Duration::from_secs(1))
        .await
        .unwrap();

    // Get token should refresh and update cache
    let new_token = manager.get_token(&cred_id).await.unwrap();
    assert_ne!(new_token.token.expose(), "expired-token");

    // Cache should now have the new token
    let cached_token = cache.get(cred_id.as_str()).await.unwrap();
    assert!(cached_token.is_some());
    assert_eq!(
        cached_token.unwrap().token.expose(),
        new_token.token.expose()
    );
}

#[tokio::test]
async fn test_negative_cache_prevents_repeated_failures() {
    use nebula_credential::core::CredentialId;

    let manager = create_test_manager().await;

    let fake_id = CredentialId::from_string("nonexistent");

    // First call - should fail and cache the error
    let result1 = manager.get_token(&fake_id).await;
    assert!(result1.is_err());

    // Second call - should return cached error (no storage lookup)
    let result2 = manager.get_token(&fake_id).await;
    assert!(result2.is_err());

    // Both errors should be the same type
    assert!(matches!(
        result1,
        Err(nebula_credential::core::CredentialError::NotFound { .. })
    ));
    assert!(matches!(
        result2,
        Err(nebula_credential::core::CredentialError::NotFound { .. })
    ));
}

#[tokio::test]
async fn test_manager_without_cache() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    // Create manager without cache (degraded mode)
    let store = Arc::new(MockStateStore::new());
    let lock = MockLock::new();
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store)
        .with_lock(lock)
        .with_registry(registry)
        .build()
        .unwrap();

    // Should still work without cache
    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    let token = manager.get_token(&cred_id).await.unwrap();
    assert!(!token.is_expired());
}

#[tokio::test]
async fn test_cache_statistics_tracking() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let cache = Arc::new(MockTokenCache::new());
    let store = Arc::new(MockStateStore::new());
    let lock = MockLock::new();
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store)
        .with_lock(lock)
        .with_cache(Arc::clone(&cache) as Arc<dyn nebula_credential::traits::TokenCache>)
        .with_registry(registry)
        .build()
        .unwrap();

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    let initial_stats = cache.stats();

    // Hit the cache multiple times
    for _ in 0..3 {
        let _ = manager.get_token(&cred_id).await;
    }

    let final_stats = cache.stats();

    // Should have recorded hits
    assert!(final_stats.hits > initial_stats.hits);
}

#[tokio::test]
async fn test_cache_population_after_creation() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    // Cache should be populated immediately after creation
    let cache = manager.cache().unwrap();
    let cached_token = cache.get(cred_id.as_str()).await.unwrap();

    assert!(
        cached_token.is_some(),
        "cache should contain token after creation"
    );
}
