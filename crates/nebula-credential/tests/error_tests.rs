//! Integration tests for error handling scenarios

use nebula_credential::core::CredentialId;
use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache, TestCredentialFactory};
use nebula_credential::traits::StateStore;
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde_json::json;
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
async fn test_unknown_credential_type_error() {
    let manager = create_test_manager().await;
    // Don't register any factories

    let result = manager
        .create_credential("unknown_type", json!({"value": "test"}))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        nebula_credential::core::CredentialError::TypeNotRegistered { .. }
    ));
}

#[tokio::test]
async fn test_get_nonexistent_credential() {
    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    let result = manager.get_token(&CredentialId::new()).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        nebula_credential::core::CredentialError::NotFound { .. }
    ));
}

#[tokio::test]
async fn test_credential_initialization_failure() {
    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Create credential with should_fail = true
    let result = manager
        .create_credential(
            "test_credential",
            json!({
                "value": "test",
                "should_fail": true
            }),
        )
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        nebula_credential::core::CredentialError::InvalidInput { .. }
    ));
}

#[tokio::test]
async fn test_credential_refresh_failure() {
    use nebula_credential::core::AccessToken;
    use nebula_credential::testing::TestCredential;
    use nebula_credential::traits::TokenCache;
    use std::time::{Duration, SystemTime};

    // Create manager with TestCredential that fails on refresh
    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    let failing_credential = TestCredential {
        fail_on_refresh: true,
        refresh_delay: None,
    };

    registry.register(Arc::new(TestCredentialFactory::with_credential(
        failing_credential,
    )));

    let manager = CredentialManager::builder()
        .with_store(store)
        .with_lock(MockLock::new())
        .with_cache(Arc::clone(&cache) as Arc<dyn TokenCache>)
        .with_registry(registry)
        .build()
        .unwrap();

    // First create credential successfully (init doesn't fail)
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

    // Put expired token to force refresh
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

    // Get token should fail because refresh fails
    let result = manager.get_token(&cred_id).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        nebula_credential::core::CredentialError::RefreshFailed { .. }
    ));
}

#[tokio::test]
async fn test_delete_nonexistent_credential() {
    let manager = create_test_manager().await;

    // Delete is idempotent - deleting nonexistent credential succeeds
    let result = manager.delete_credential(&CredentialId::new()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_manager_without_cache_works() {
    // Manager should work in degraded mode without cache
    let store = Arc::new(MockStateStore::new());
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(store)
        .with_lock(MockLock::new())
        // No cache!
        .with_registry(registry)
        .build()
        .unwrap();

    // Should still be able to create and get tokens
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

    let token = manager.get_token(&cred_id).await.unwrap();
    assert!(!token.is_expired());
}

#[tokio::test]
async fn test_manager_requires_lock() {
    // Manager requires a distributed lock - building without it should fail
    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let result = CredentialManager::builder()
        .with_store(store)
        .with_cache(cache)
        // No lock!
        .with_registry(registry)
        .build();

    assert!(result.is_err());
    if let Err(err) = result {
        assert!(err.to_string().contains("DistributedLock"));
    }
}

#[tokio::test]
async fn test_invalid_json_deserialization() {
    let manager = create_test_manager().await;
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // Missing required field
    let result = manager
        .create_credential("test_credential", json!({"wrong_field": "value"}))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        nebula_credential::core::CredentialError::DeserializationFailed(_)
    ));
}

#[tokio::test]
async fn test_storage_failure_on_load() {
    let store = Arc::new(MockStateStore::new());
    let registry = Arc::new(CredentialRegistry::new());
    registry.register(Arc::new(TestCredentialFactory::new()));

    let manager = CredentialManager::builder()
        .with_store(Arc::clone(&store) as Arc<dyn StateStore>)
        .with_lock(MockLock::new())
        .with_cache(Arc::new(MockTokenCache::new()))
        .with_registry(registry)
        .build()
        .unwrap();

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

    // Make store fail on next load
    store.fail_next_load();

    // Clear cache to force load from storage
    manager.cache().unwrap().clear().await.unwrap();

    // Get token should fail due to storage failure
    let result = manager.get_token(&cred_id).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        nebula_credential::core::CredentialError::StorageFailed { .. }
    ));
}

#[tokio::test]
async fn test_multiple_error_types() {
    let manager = create_test_manager().await;

    // NotFound error
    let result1 = manager.get_token(&CredentialId::new()).await;
    assert!(matches!(
        result1.unwrap_err(),
        nebula_credential::core::CredentialError::NotFound { .. }
    ));

    // Register factory for next tests
    manager
        .registry()
        .register(Arc::new(TestCredentialFactory::new()));

    // InvalidInput error
    let result2 = manager
        .create_credential(
            "test_credential",
            json!({"value": "test", "should_fail": true}),
        )
        .await;
    assert!(matches!(
        result2.unwrap_err(),
        nebula_credential::core::CredentialError::InvalidInput { .. }
    ));

    // DeserializationFailed error
    let result3 = manager
        .create_credential("test_credential", json!({"missing_fields": true}))
        .await;
    assert!(matches!(
        result3.unwrap_err(),
        nebula_credential::core::CredentialError::DeserializationFailed(_)
    ));
}
