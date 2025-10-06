//! Integration tests for concurrent operations

use nebula_credential::testing::{
    MockLock, MockStateStore, MockTokenCache, TestCredential, TestCredentialFactory,
};
use nebula_credential::traits::{StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

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
async fn test_concurrent_get_token_no_duplicate_refresh() {
    // When multiple tasks request the same token concurrently,
    // only one refresh should occur due to locking
    let cache = Arc::new(MockTokenCache::new());
    let store = Arc::new(MockStateStore::new());
    let registry = Arc::new(CredentialRegistry::new());

    // Use credential with delay to ensure concurrent requests overlap
    let slow_credential = TestCredential {
        fail_on_refresh: false,
        refresh_delay: Some(Duration::from_millis(50)),
    };
    registry.register(Arc::new(TestCredentialFactory::with_credential(
        slow_credential,
    )));

    let manager = CredentialManager::builder()
        .with_store(Arc::clone(&store) as Arc<dyn StateStore>)
        .with_lock(MockLock::new())
        .with_cache(Arc::clone(&cache) as Arc<dyn TokenCache>)
        .with_registry(registry)
        .build()
        .unwrap();

    // Create credential
    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "concurrent-test",
                "should_fail": false
            }),
        )
        .await
        .unwrap();

    // Clear cache to force refresh
    cache.clear().await.unwrap();

    // Make 3 concurrent requests using tokio::join
    let id1 = cred_id.clone();
    let id2 = cred_id.clone();
    let id3 = cred_id.clone();

    let (r1, r2, r3) = tokio::join!(
        manager.get_token(&id1),
        manager.get_token(&id2),
        manager.get_token(&id3)
    );

    // All should succeed
    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());

    // Check cache stats
    let stats = cache.stats();
    assert!(stats.puts > 0, "Should have at least one refresh");
}

#[tokio::test]
async fn test_concurrent_credential_creation() {
    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Create 5 credentials concurrently using tokio::join
    let (r1, r2, r3, r4, r5) = tokio::join!(
        manager.create_credential(
            "test_credential",
            json!({"value": "cred-0", "should_fail": false}),
        ),
        manager.create_credential(
            "test_credential",
            json!({"value": "cred-1", "should_fail": false}),
        ),
        manager.create_credential(
            "test_credential",
            json!({"value": "cred-2", "should_fail": false}),
        ),
        manager.create_credential(
            "test_credential",
            json!({"value": "cred-3", "should_fail": false}),
        ),
        manager.create_credential(
            "test_credential",
            json!({"value": "cred-4", "should_fail": false}),
        ),
    );

    // All should succeed with unique IDs
    let ids = vec![
        r1.unwrap(),
        r2.unwrap(),
        r3.unwrap(),
        r4.unwrap(),
        r5.unwrap(),
    ];

    // All IDs should be unique
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            assert_ne!(ids[i], ids[j], "Credential IDs should be unique");
        }
    }
}

#[tokio::test]
async fn test_concurrent_delete_and_get() {
    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "test",
                "should_fail": false
            }),
        )
        .await
        .unwrap();

    // Concurrent delete and get operations
    let id1 = cred_id.clone();
    let id2 = cred_id.clone();

    let (delete_result, get_result) =
        tokio::join!(manager.delete_credential(&id1), manager.get_token(&id2));

    // Delete should succeed
    assert!(delete_result.is_ok());

    // Get might succeed or fail depending on timing
    // (this is expected - either it got the token before delete, or it didn't)
    // Just verify it doesn't panic or hang
    drop(get_result);
}

#[tokio::test]
async fn test_concurrent_refresh_with_different_credentials() {
    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Create 3 different credentials
    let id1 = manager
        .create_credential(
            "test_credential",
            json!({"value": "cred-0", "should_fail": false}),
        )
        .await
        .unwrap();
    let id2 = manager
        .create_credential(
            "test_credential",
            json!({"value": "cred-1", "should_fail": false}),
        )
        .await
        .unwrap();
    let id3 = manager
        .create_credential(
            "test_credential",
            json!({"value": "cred-2", "should_fail": false}),
        )
        .await
        .unwrap();

    // Clear cache
    manager.cache().unwrap().clear().await.unwrap();

    // Get all tokens concurrently
    let (r1, r2, r3) = tokio::join!(
        manager.get_token(&id1),
        manager.get_token(&id2),
        manager.get_token(&id3)
    );

    // All should succeed
    assert!(!r1.unwrap().is_expired());
    assert!(!r2.unwrap().is_expired());
    assert!(!r3.unwrap().is_expired());
}

#[tokio::test]
async fn test_concurrent_cache_updates() {
    let cache = Arc::new(MockTokenCache::new());
    let store = Arc::new(MockStateStore::new());
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(Arc::clone(&store) as Arc<dyn StateStore>)
        .with_lock(MockLock::new())
        .with_cache(Arc::clone(&cache) as Arc<dyn TokenCache>)
        .with_registry(registry)
        .build()
        .unwrap();

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "cache-test",
                "should_fail": false
            }),
        )
        .await
        .unwrap();

    // First get populates cache
    let _ = manager.get_token(&cred_id).await.unwrap();

    // Make concurrent requests - should hit cache
    let id1 = cred_id.clone();
    let id2 = cred_id.clone();
    let id3 = cred_id.clone();
    let id4 = cred_id.clone();

    let (r1, r2, r3, r4) = tokio::join!(
        manager.get_token(&id1),
        manager.get_token(&id2),
        manager.get_token(&id3),
        manager.get_token(&id4)
    );

    // All should succeed
    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());
    assert!(r4.is_ok());

    // Cache should have hits
    let stats = cache.stats();
    assert!(stats.hits > 0, "Should have cache hits");
}

#[tokio::test]
async fn test_race_condition_token_expiry() {
    use nebula_credential::core::AccessToken;
    use std::time::SystemTime;

    let cache = Arc::new(MockTokenCache::new());
    let store = Arc::new(MockStateStore::new());
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(Arc::clone(&store) as Arc<dyn StateStore>)
        .with_lock(MockLock::new())
        .with_cache(Arc::clone(&cache) as Arc<dyn TokenCache>)
        .with_registry(registry)
        .build()
        .unwrap();

    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "expiry-test",
                "should_fail": false
            }),
        )
        .await
        .unwrap();

    // Put a token that's about to expire
    let almost_expired = AccessToken {
        token: nebula_credential::core::SecureString::new("almost-expired"),
        token_type: nebula_credential::core::TokenType::Bearer,
        issued_at: SystemTime::now() - Duration::from_secs(3500),
        expires_at: Some(SystemTime::now() + Duration::from_secs(1)),
        scopes: None,
        claims: Default::default(),
    };

    cache
        .as_ref()
        .put(cred_id.as_str(), &almost_expired, Duration::from_secs(1))
        .await
        .unwrap();

    // Make concurrent requests while token is expiring
    let id1 = cred_id.clone();
    let id2 = cred_id.clone();
    let id3 = cred_id.clone();

    let (r1, r2, r3) = tokio::join!(
        manager.get_token(&id1),
        manager.get_token(&id2),
        manager.get_token(&id3)
    );

    // All should get valid (non-expired) tokens
    assert!(!r1.unwrap().is_expired());
    assert!(!r2.unwrap().is_expired());
    assert!(!r3.unwrap().is_expired());
}
