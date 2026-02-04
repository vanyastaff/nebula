//! Integration tests for retrieve_scoped() and list_scoped() methods
//!
//! Tests Phase 4: Scope enforcement in retrieve and list operations

use nebula_credential::core::ManagerError;
use nebula_credential::prelude::*;
use std::sync::Arc;

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

/// T049: retrieve_scoped() with exact scope match
#[tokio::test]
async fn test_retrieve_scoped_exact_match() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("db-cred").unwrap();
    let data = create_test_data("password");
    let metadata = CredentialMetadata::new();

    // Store with scope
    let context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:eng")
        .unwrap();

    manager
        .store(&id, data.clone(), metadata, &context)
        .await
        .unwrap();

    // Retrieve with exact same scope - should succeed
    let result = manager.retrieve_scoped(&id, &context).await.unwrap();
    assert!(result.is_some());

    let (_, retrieved_metadata) = result.unwrap();
    assert_eq!(
        retrieved_metadata.scope.as_ref().map(|s| s.as_str()),
        Some("org:acme/team:eng")
    );
}

/// T049: retrieve_scoped() with hierarchical scope match (parent accessing child)
#[tokio::test]
async fn test_retrieve_scoped_hierarchical_match() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("service-cred").unwrap();
    let data = create_test_data("api-key");
    let metadata = CredentialMetadata::new();

    // Store with child scope
    let child_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:eng/service:api")
        .unwrap();

    manager
        .store(&id, data, metadata, &child_context)
        .await
        .unwrap();

    // Retrieve with parent scope - should succeed
    let parent_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:eng")
        .unwrap();

    let result = manager.retrieve_scoped(&id, &parent_context).await.unwrap();
    assert!(
        result.is_some(),
        "Parent scope should access child credential"
    );
}

/// T049: retrieve_scoped() with scope mismatch - should return None
#[tokio::test]
async fn test_retrieve_scoped_mismatch() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("team-a-cred").unwrap();
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();

    // Store with team-a scope
    let team_a_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:a")
        .unwrap();

    manager
        .store(&id, data, metadata, &team_a_context)
        .await
        .unwrap();

    // Try to retrieve with team-b scope - should return None (access denied)
    let team_b_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:b")
        .unwrap();

    let result = manager.retrieve_scoped(&id, &team_b_context).await.unwrap();
    assert!(result.is_none(), "Cross-scope access should be denied");
}

/// T049: retrieve_scoped() without scope in context - should return error
#[tokio::test]
async fn test_retrieve_scoped_no_context_scope() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("some-cred").unwrap();

    // Context without scope
    let context = CredentialContext::new("user-1");

    let result = manager.retrieve_scoped(&id, &context).await;
    assert!(result.is_err(), "Should require scope in context");

    match result.unwrap_err() {
        ManagerError::ScopeRequired { operation } => {
            assert_eq!(operation, "retrieve_scoped");
        }
        e => panic!("Expected ScopeRequired error, got: {:?}", e),
    }
}

/// T049: retrieve_scoped() with unscoped credential - should return None
#[tokio::test]
async fn test_retrieve_scoped_unscoped_credential() {
    let manager = create_test_manager().await;
    let id = CredentialId::new("legacy-cred").unwrap();
    let data = create_test_data("legacy-secret");
    let metadata = CredentialMetadata::new();

    // Store without scope (legacy credential)
    let unscoped_context = CredentialContext::new("user-1");
    manager
        .store(&id, data, metadata, &unscoped_context)
        .await
        .unwrap();

    // Try to retrieve with scoped context - should return None
    let scoped_context = CredentialContext::new("user-1")
        .with_scope("org:acme")
        .unwrap();

    let result = manager.retrieve_scoped(&id, &scoped_context).await.unwrap();
    assert!(
        result.is_none(),
        "Unscoped credentials not accessible via retrieve_scoped"
    );
}

/// T050: list_scoped() filters by exact scope
#[tokio::test]
async fn test_list_scoped_exact_match() {
    let manager = create_test_manager().await;
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();

    let scope_a_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:a")
        .unwrap();
    let scope_b_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:b")
        .unwrap();

    // Store credentials in different scopes
    let cred_a1 = CredentialId::new("team-a-db").unwrap();
    let cred_a2 = CredentialId::new("team-a-api").unwrap();
    let cred_b1 = CredentialId::new("team-b-db").unwrap();

    manager
        .store(&cred_a1, data.clone(), metadata.clone(), &scope_a_context)
        .await
        .unwrap();
    manager
        .store(&cred_a2, data.clone(), metadata.clone(), &scope_a_context)
        .await
        .unwrap();
    manager
        .store(&cred_b1, data.clone(), metadata.clone(), &scope_b_context)
        .await
        .unwrap();

    // List with team-a scope - should only see team-a credentials
    let a_creds = manager.list_scoped(&scope_a_context).await.unwrap();
    assert_eq!(a_creds.len(), 2, "Should see 2 team-a credentials");
    assert!(a_creds.contains(&cred_a1));
    assert!(a_creds.contains(&cred_a2));

    // List with team-b scope - should only see team-b credential
    let b_creds = manager.list_scoped(&scope_b_context).await.unwrap();
    assert_eq!(b_creds.len(), 1, "Should see 1 team-b credential");
    assert!(b_creds.contains(&cred_b1));
}

/// T050: list_scoped() includes hierarchical children
#[tokio::test]
async fn test_list_scoped_hierarchical() {
    let manager = create_test_manager().await;
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();

    // Create parent and child scopes
    let parent_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:eng")
        .unwrap();
    let child1_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:eng/service:api")
        .unwrap();
    let child2_context = CredentialContext::new("user-1")
        .with_scope("org:acme/team:eng/service:db")
        .unwrap();

    // Store credentials at different levels
    let parent_cred = CredentialId::new("team-key").unwrap();
    let child1_cred = CredentialId::new("api-key").unwrap();
    let child2_cred = CredentialId::new("db-password").unwrap();

    manager
        .store(
            &parent_cred,
            data.clone(),
            metadata.clone(),
            &parent_context,
        )
        .await
        .unwrap();
    manager
        .store(
            &child1_cred,
            data.clone(),
            metadata.clone(),
            &child1_context,
        )
        .await
        .unwrap();
    manager
        .store(
            &child2_cred,
            data.clone(),
            metadata.clone(),
            &child2_context,
        )
        .await
        .unwrap();

    // List with parent scope - should see all 3 credentials
    let all_creds = manager.list_scoped(&parent_context).await.unwrap();
    assert_eq!(
        all_creds.len(),
        3,
        "Parent scope should see all child credentials"
    );
    assert!(all_creds.contains(&parent_cred));
    assert!(all_creds.contains(&child1_cred));
    assert!(all_creds.contains(&child2_cred));

    // List with child1 scope - should only see child1
    let child1_creds = manager.list_scoped(&child1_context).await.unwrap();
    assert_eq!(child1_creds.len(), 1, "Child scope should see only itself");
    assert!(child1_creds.contains(&child1_cred));
}

/// T050: list_scoped() without scope in context - should return error
#[tokio::test]
async fn test_list_scoped_no_context_scope() {
    let manager = create_test_manager().await;

    // Context without scope
    let context = CredentialContext::new("user-1");

    let result = manager.list_scoped(&context).await;
    assert!(result.is_err(), "Should require scope in context");

    match result.unwrap_err() {
        ManagerError::ScopeRequired { operation } => {
            assert_eq!(operation, "list_scoped");
        }
        e => panic!("Expected ScopeRequired error, got: {:?}", e),
    }
}

/// T051: list_scoped() excludes unscoped credentials
#[tokio::test]
async fn test_list_scoped_excludes_unscoped() {
    let manager = create_test_manager().await;
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();

    let scoped_context = CredentialContext::new("user-1")
        .with_scope("org:acme")
        .unwrap();
    let unscoped_context = CredentialContext::new("user-1");

    // Store scoped and unscoped credentials
    let scoped_cred = CredentialId::new("scoped").unwrap();
    let unscoped_cred = CredentialId::new("legacy").unwrap();

    manager
        .store(
            &scoped_cred,
            data.clone(),
            metadata.clone(),
            &scoped_context,
        )
        .await
        .unwrap();
    manager
        .store(
            &unscoped_cred,
            data.clone(),
            metadata.clone(),
            &unscoped_context,
        )
        .await
        .unwrap();

    // List with scoped context - should only see scoped credential
    let scoped_list = manager.list_scoped(&scoped_context).await.unwrap();
    assert_eq!(scoped_list.len(), 1, "Should only see scoped credential");
    assert!(scoped_list.contains(&scoped_cred));
    assert!(
        !scoped_list.contains(&unscoped_cred),
        "Should not see unscoped credential"
    );
}
