//! Integration tests for CredentialManager

use nebula_credential::core::CredentialId;
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
async fn test_manager_initialization() {
    let _manager = create_test_manager().await;
    // Manager should be created successfully
}

#[tokio::test]
async fn test_create_and_get_token_workflow() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let manager = create_test_manager().await;

    // Register test credential type
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Create credential
    let input = json!({
        "value": "test-secret",
        "should_fail": false
    });

    let cred_id = manager
        .create_credential("test_credential", input)
        .await
        .expect("credential creation should succeed");

    // Get token (should be cached from creation)
    let token = manager
        .get_token(&cred_id)
        .await
        .expect("get_token should succeed");

    assert!(!token.is_expired());
    assert_eq!(token.token_type, nebula_credential::core::TokenType::Bearer);
}

#[tokio::test]
async fn test_token_refresh_when_expired() {
    use nebula_credential::testing::TestCredentialFactory;
    use nebula_credential::core::AccessToken;
    use serde_json::json;
    use std::time::{Duration, SystemTime};

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Create credential with expired token
    let cred_id = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": false}),
        )
        .await
        .unwrap();

    // Manually insert expired token in cache to simulate expiration
    if let Some(cache) = manager.cache() {
        let expired_token = AccessToken {
            token: nebula_credential::core::SecureString::new("expired"),
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
    }

    // Get token should trigger refresh
    let token = manager.get_token(&cred_id).await.unwrap();
    assert!(!token.is_expired(), "refreshed token should not be expired");
}

#[tokio::test]
async fn test_credential_deletion() {
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

    // Verify credential exists
    let token = manager.get_token(&cred_id).await;
    assert!(token.is_ok());

    // Delete credential
    manager.delete_credential(&cred_id).await.unwrap();

    // Verify credential is gone
    let result = manager.get_token(&cred_id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_credential_id_error() {
    let manager = create_test_manager().await;

    let fake_id = CredentialId::from_string("nonexistent");
    let result = manager.get_token(&fake_id).await;

    assert!(matches!(
        result,
        Err(nebula_credential::core::CredentialError::NotFound { .. })
    ));
}

#[tokio::test]
async fn test_unknown_credential_type_error() {
    use serde_json::json;

    let manager = create_test_manager().await;

    let result = manager
        .create_credential("unknown_type", json!({}))
        .await;

    assert!(matches!(
        result,
        Err(nebula_credential::core::CredentialError::TypeNotRegistered { .. })
    ));
}

#[tokio::test]
async fn test_list_credentials() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Initially empty
    let list = manager.list_credentials().await.unwrap();
    assert_eq!(list.len(), 0);

    // Create credentials
    let _id1 = manager
        .create_credential(
            "test_credential",
            json!({"value": "test1", "should_fail": false}),
        )
        .await
        .unwrap();

    let _id2 = manager
        .create_credential(
            "test_credential",
            json!({"value": "test2", "should_fail": false}),
        )
        .await
        .unwrap();

    // Should have 2 credentials
    let list = manager.list_credentials().await.unwrap();
    assert_eq!(list.len(), 2);
}

#[tokio::test]
async fn test_cache_is_populated_after_creation() {
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

    // Cache should have the token
    if let Some(cache) = manager.cache() {
        let cached = cache.get(cred_id.as_str()).await.unwrap();
        assert!(cached.is_some(), "token should be in cache after creation");
    }
}

#[tokio::test]
async fn test_credential_creation_failure() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Try to create with should_fail=true
    let result = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": true}),
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_state_persistence_across_manager_restart() {
    use nebula_credential::testing::TestCredentialFactory;
    use serde_json::json;

    // Create first manager and credential
    let store: Arc<dyn nebula_credential::traits::StateStore> =
        Arc::new(nebula_credential::testing::MockStateStore::new());
    let lock = nebula_credential::testing::MockLock::new();
    let cache: Arc<dyn nebula_credential::traits::TokenCache> =
        Arc::new(nebula_credential::testing::MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager1 = CredentialManager::builder()
        .with_store(Arc::clone(&store))
        .with_lock(lock)
        .with_cache(Arc::clone(&cache))
        .with_registry(Arc::clone(&registry))
        .build()
        .unwrap();

    let cred_id = manager1
        .create_credential(
            "test_credential",
            json!({"value": "persistent", "should_fail": false}),
        )
        .await
        .unwrap();

    // Create second manager with same store (simulating restart)
    let lock2 = nebula_credential::testing::MockLock::new();
    let cache2: Arc<dyn nebula_credential::traits::TokenCache> =
        Arc::new(nebula_credential::testing::MockTokenCache::new());

    let manager2 = CredentialManager::builder()
        .with_store(store)
        .with_lock(lock2)
        .with_cache(cache2)
        .with_registry(registry)
        .build()
        .unwrap();

    // Should still be able to get token from restarted manager
    let token = manager2
        .get_token(&cred_id)
        .await
        .expect("should get token after restart");
    assert!(!token.is_expired());
}
