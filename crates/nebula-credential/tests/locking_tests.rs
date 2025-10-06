//! Integration tests for distributed locking behavior

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
async fn test_lock_initialization() {
    let _manager = create_test_manager().await;
    // Manager with lock should be created successfully
}

#[tokio::test]
async fn test_concurrent_token_requests() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let manager = Arc::new(create_test_manager().await);
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

    // Clear cache to force refresh
    manager.cache().unwrap().clear().await.unwrap();

    // Make concurrent requests - lock should serialize them
    let manager1 = Arc::clone(&manager);
    let manager2 = Arc::clone(&manager);
    let cred_id1 = cred_id.clone();
    let cred_id2 = cred_id.clone();

    let (result1, result2) =
        tokio::join!(manager1.get_token(&cred_id1), manager2.get_token(&cred_id2));

    // Both should succeed despite concurrent access
    assert!(result1.is_ok());
    assert!(result2.is_ok());
}

#[tokio::test]
async fn test_lock_release_allows_subsequent_access() {
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

    // First call
    let _ = manager.get_token(&cred_id).await.unwrap();

    // Second call should work (lock was released)
    let _ = manager.get_token(&cred_id).await.unwrap();

    // Third call should also work
    let result = manager.get_token(&cred_id).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_cache_recheck_after_lock() {
    use nebula_credential::core::AccessToken;
    use nebula_credential::testing::TestCredentialFactory;
    use nebula_credential::traits::TokenCache;
    use serde_json::json;
    use std::time::{Duration, SystemTime};

    let cache = Arc::new(MockTokenCache::new());
    let store = Arc::new(MockStateStore::new());
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store)
        .with_lock(MockLock::new())
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

    // Put expired token to trigger refresh
    let expired_token = AccessToken {
        token: nebula_credential::core::SecureString::new("expired"),
        token_type: nebula_credential::core::TokenType::Bearer,
        issued_at: SystemTime::now() - Duration::from_secs(7200),
        expires_at: Some(SystemTime::now() - Duration::from_secs(1)),
        scopes: None,
        claims: Default::default(),
    };

    cache
        .as_ref()
        .put(cred_id.as_str(), &expired_token, Duration::from_secs(1))
        .await
        .unwrap();

    // Get token - should refresh and update cache
    let token = manager.get_token(&cred_id).await.unwrap();

    // Should get fresh token from refresh
    assert!(!token.is_expired());
    assert_ne!(token.token.expose(), "expired");
}

#[tokio::test]
async fn test_multiple_credentials_isolated_locks() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Create two different credentials
    let cred_id1 = manager
        .create_credential(
            "test_credential",
            json!({"value": "cred1", "should_fail": false}),
        )
        .await
        .unwrap();

    let cred_id2 = manager
        .create_credential(
            "test_credential",
            json!({"value": "cred2", "should_fail": false}),
        )
        .await
        .unwrap();

    // Access both credentials - each should use separate lock
    let token1 = manager.get_token(&cred_id1).await.unwrap();
    let token2 = manager.get_token(&cred_id2).await.unwrap();

    assert!(!token1.is_expired());
    assert!(!token2.is_expired());
}
