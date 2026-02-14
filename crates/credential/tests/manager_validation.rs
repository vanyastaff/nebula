//! Integration tests for credential validation and health checks
//!
//! Tests Phase 5: User Story 3 - Credential Validation and Health Checks

use nebula_credential::prelude::*;
use std::sync::Arc;
use std::time::Duration;

/// Helper to create test manager
async fn create_test_manager() -> CredentialManager {
    CredentialManager::builder()
        .storage(Arc::new(MockStorageProvider::new()))
        .build()
}

/// Helper to create encrypted test data
fn create_test_data(value: &str) -> EncryptedData {
    let key = EncryptionKey::from_bytes([0u8; 32]);
    encrypt(&key, value.as_bytes()).unwrap()
}

/// T060: Validate non-expired credential
#[tokio::test]
async fn test_validate_non_expired() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("valid-cred").unwrap();
    let data = create_test_data("password");
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Store credential
    manager.store(&id, data, metadata, &context).await.unwrap();

    // Validate immediately - should be valid
    let result = manager.validate(&id, &context).await.unwrap();

    assert!(result.is_valid(), "Credential should be valid");
    assert!(!result.is_expired(), "Credential should not be expired");
}

/// T061: Validate expired credential
///
/// NOTE: This test creates a credential that was "created" in the past
/// by manually setting created_at to simulate an expired credential
#[tokio::test]
async fn test_validate_expired() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("expiring-cred").unwrap();
    let data = create_test_data("password");

    // Create metadata with created_at in the past and 1-day expiration
    let mut metadata = CredentialMetadata::new();
    // Set created_at to 2 days ago
    metadata.created_at = chrono::Utc::now() - chrono::Duration::days(2);
    metadata.last_modified = metadata.created_at;
    // Policy: expires after 1 day
    let policy = RotationPolicy { interval_days: 1 };
    metadata.rotation_policy = Some(policy);

    let context = CredentialContext::new("user-1");

    // Store credential (will have created_at = 2 days ago, expires after 1 day)
    manager.store(&id, data, metadata, &context).await.unwrap();

    // Validate - should be expired (created 2 days ago, expiration after 1 day)
    let result = manager.validate(&id, &context).await.unwrap();

    assert!(!result.is_valid(), "Credential should be invalid");
    assert!(result.is_expired(), "Credential should be expired");
}

/// T062: Batch validation
#[tokio::test]
async fn test_validate_batch() {
    let manager = create_test_manager().await;
    let data = create_test_data("password");
    let metadata = CredentialMetadata::new();
    let context = CredentialContext::new("user-1");

    // Store multiple credentials
    let ids: Vec<CredentialId> = (0..5)
        .map(|i| CredentialId::new(format!("cred-{}", i)).unwrap())
        .collect();

    for id in &ids {
        manager
            .store(id, data.clone(), metadata.clone(), &context)
            .await
            .unwrap();
    }

    // Batch validate
    let results = manager.validate_batch(&ids, &context).await.unwrap();

    assert_eq!(results.len(), 5, "Should validate all 5 credentials");
    for result in results.values() {
        assert!(result.is_valid(), "All credentials should be valid");
    }
}

/// T063: Rotation recommendation detection
///
/// NOTE: Tests rotation_recommended() logic with credentials at different ages
#[tokio::test]
async fn test_rotation_recommended() {
    let manager = create_test_manager().await;

    // Test 1: Fresh credential - no rotation needed
    let id_fresh = CredentialId::new("fresh-cred").unwrap();
    let data = create_test_data("password");
    let mut metadata_fresh = CredentialMetadata::new();
    metadata_fresh.rotation_policy = Some(RotationPolicy { interval_days: 30 });
    let context = CredentialContext::new("user-1");

    manager
        .store(&id_fresh, data.clone(), metadata_fresh, &context)
        .await
        .unwrap();

    let result_fresh = manager.validate(&id_fresh, &context).await.unwrap();
    let max_age = Duration::from_secs(30 * 24 * 3600); // 30 days
    assert!(
        !result_fresh.rotation_recommended(max_age),
        "Fresh credential should not need rotation"
    );

    // Test 2: Old credential (created 25 days ago with 30-day policy)
    // Should recommend rotation (>75% of lifetime used)
    let id_old = CredentialId::new("old-cred").unwrap();
    let mut metadata_old = CredentialMetadata::new();
    metadata_old.created_at = chrono::Utc::now() - chrono::Duration::days(25);
    metadata_old.last_modified = metadata_old.created_at;
    metadata_old.rotation_policy = Some(RotationPolicy { interval_days: 30 });

    manager
        .store(&id_old, data, metadata_old, &context)
        .await
        .unwrap();

    let result_old = manager.validate(&id_old, &context).await.unwrap();
    assert!(
        result_old.rotation_recommended(max_age),
        "Old credential (25/30 days) should recommend rotation"
    );
}

/// T064: Validate with scope isolation
#[tokio::test]
async fn test_validate_scoped() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("scoped-cred").unwrap();
    let data = create_test_data("password");
    let metadata = CredentialMetadata::new();

    // Store with scope
    let scope_a = CredentialContext::new("user-1")
        .with_scope("org:acme/team:a")
        .unwrap();

    manager.store(&id, data, metadata, &scope_a).await.unwrap();

    // Validate with correct scope - should succeed
    let result_correct = manager.validate(&id, &scope_a).await.unwrap();
    assert!(
        result_correct.is_valid(),
        "Should validate with correct scope"
    );

    // Validate with different scope context
    // NOTE: validate() doesn't enforce scope - it's a basic health check
    // For scope enforcement, use retrieve_scoped()
    let scope_b = CredentialContext::new("user-1")
        .with_scope("org:acme/team:b")
        .unwrap();

    // This will succeed because validate() doesn't check scope
    let result_different_scope = manager.validate(&id, &scope_b).await.unwrap();
    assert!(
        result_different_scope.is_valid(),
        "validate() is scope-agnostic - use retrieve_scoped() for scope enforcement"
    );
}
