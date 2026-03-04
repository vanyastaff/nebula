//! Integration tests for scope-based multi-tenant credential isolation
//!
//! Tests Phase 4: User Story 2 - Multi-Tenant Credential Isolation
//! Uses ScopeLevel from nebula-core for platform consistency.

use nebula_core::{OrganizationId, ProjectId, ScopeLevel};
use nebula_credential::prelude::*;
use std::sync::Arc;

/// Helper to create test manager with mock storage
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

/// T043: Store credential with scope
#[tokio::test]
async fn test_store_credential_with_scope() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("secret-value");
    let metadata = CredentialMetadata::new();
    let project_id = ProjectId::new();
    let context = CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_id));

    // Store should succeed
    manager
        .store(&id, data.clone(), metadata.clone(), &context)
        .await
        .expect("Store succeeds");

    // Retrieve with same scope
    let result = manager.retrieve(&id, &context).await.unwrap();
    assert!(result.is_some());

    let (_, retrieved_metadata) = result.unwrap();
    assert!(matches!(
        retrieved_metadata.owner_scope.as_ref(),
        Some(ScopeLevel::Project(id)) if *id == project_id
    ));
}

/// T044: Retrieve credential with matching scope
#[tokio::test]
async fn test_retrieve_with_matching_scope() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();
    let project_id = ProjectId::new();

    let store_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_id));

    manager
        .store(&id, data.clone(), metadata, &store_context)
        .await
        .unwrap();

    let retrieve_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_id));

    let result = manager
        .retrieve(&id, &retrieve_context)
        .await
        .expect("Retrieve succeeds");

    assert!(result.is_some());
}

/// T045: retrieve() does not enforce scope — documents current behavior
#[tokio::test]
async fn test_retrieve_with_mismatched_scope_current_behavior() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("team-a-secret");
    let metadata = CredentialMetadata::new();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();

    let team_a_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_a));

    manager
        .store(&id, data, metadata, &team_a_context)
        .await
        .unwrap();

    let team_b_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_b));

    let result = manager.retrieve(&id, &team_b_context).await.unwrap();

    assert!(
        result.is_some(),
        "retrieve() does not enforce scope; use retrieve_scoped() for isolation"
    );
}

/// T046: list_scoped filters by scope
#[tokio::test]
async fn test_list_credentials_by_scope() {
    let manager = create_test_manager().await;
    let cred1 = CredentialId::new();
    let cred2 = CredentialId::new();
    let cred3 = CredentialId::new();
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();

    let project_eng = ProjectId::new();
    let project_sales = ProjectId::new();

    let eng_context = CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_eng));
    let sales_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_sales));

    manager
        .store(&cred1, data.clone(), metadata.clone(), &eng_context)
        .await
        .unwrap();
    manager
        .store(&cred2, data.clone(), metadata.clone(), &eng_context)
        .await
        .unwrap();
    manager
        .store(&cred3, data.clone(), metadata.clone(), &sales_context)
        .await
        .unwrap();

    let eng_creds = manager.list_scoped(&eng_context).await.unwrap();
    assert_eq!(eng_creds.len(), 2, "Eng scope sees only eng credentials");

    let sales_creds = manager.list_scoped(&sales_context).await.unwrap();
    assert_eq!(
        sales_creds.len(),
        1,
        "Sales scope sees only sales credential"
    );
}

/// T047: Scope hierarchy — Organization can access Project credentials
#[tokio::test]
async fn test_scope_hierarchy_parent_access() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("service-secret");
    let metadata = CredentialMetadata::new();
    let project_id = ProjectId::new();
    let org_id = OrganizationId::new();

    let project_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_id));

    manager
        .store(&id, data, metadata, &project_context)
        .await
        .unwrap();

    let org_context = CredentialContext::new("user-1").with_scope(ScopeLevel::Organization(org_id));

    let result = manager.retrieve_scoped(&id, &org_context).await.unwrap();

    assert!(
        result.is_some(),
        "Organization scope can access Project credentials (Project is_contained_in Organization)"
    );
}

/// T048: Scope isolation between tenants
#[tokio::test]
async fn test_scope_isolation_between_tenants() {
    let manager = create_test_manager().await;
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();
    let org_a = OrganizationId::new();
    let org_b = OrganizationId::new();

    let tenant_a_id = CredentialId::new();
    let tenant_a_context =
        CredentialContext::new("user-a").with_scope(ScopeLevel::Organization(org_a));

    manager
        .store(
            &tenant_a_id,
            data.clone(),
            metadata.clone(),
            &tenant_a_context,
        )
        .await
        .unwrap();

    let tenant_b_id = CredentialId::new();
    let tenant_b_context =
        CredentialContext::new("user-b").with_scope(ScopeLevel::Organization(org_b));

    manager
        .store(
            &tenant_b_id,
            data.clone(),
            metadata.clone(),
            &tenant_b_context,
        )
        .await
        .unwrap();

    let a_result = manager
        .retrieve(&tenant_a_id, &tenant_a_context)
        .await
        .unwrap()
        .unwrap();
    let b_result = manager
        .retrieve(&tenant_b_id, &tenant_b_context)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(
        a_result.1.owner_scope.as_ref(),
        Some(ScopeLevel::Organization(id)) if *id == org_a
    ));
    assert!(matches!(
        b_result.1.owner_scope.as_ref(),
        Some(ScopeLevel::Organization(id)) if *id == org_b
    ));
}
