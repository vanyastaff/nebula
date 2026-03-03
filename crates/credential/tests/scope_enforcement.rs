//! Scope enforcement tests (Phase 1: Contract Consolidation)
//!
//! Verifies retrieve_scoped, list_scoped using ScopeLevel from nebula-core.

use nebula_credential::prelude::*;
use nebula_credential::ManagerError;
use nebula_core::{OrganizationId, ProjectId, ScopeLevel};
use std::sync::Arc;

fn create_test_manager() -> CredentialManager {
    CredentialManager::builder()
        .storage(Arc::new(MockStorageProvider::new()))
        .build()
}

fn create_test_data(value: &str) -> EncryptedData {
    let key = EncryptionKey::from_bytes([0u8; 32]);
    encrypt(&key, value.as_bytes()).unwrap()
}

/// retrieve_scoped requires scope in context — returns Err(ScopeRequired) when caller_scope is None
#[tokio::test]
async fn retrieve_scoped_requires_scope() {
    let manager = create_test_manager();
    let id = CredentialId::new();
    let data = create_test_data("secret");
    let project_id = ProjectId::new();
    let ctx_with_scope =
        CredentialContext::new("user").with_scope(ScopeLevel::Project(project_id));
    manager
        .store(&id, data, CredentialMetadata::new(), &ctx_with_scope)
        .await
        .unwrap();

    let ctx_no_scope = CredentialContext::new("user");
    let result = manager.retrieve_scoped(&id, &ctx_no_scope).await;

    assert!(
        matches!(result, Err(ManagerError::ScopeRequired { .. })),
        "retrieve_scoped must return ScopeRequired when context has no scope"
    );
}

/// retrieve_scoped returns None when credential scope does not match context scope
#[tokio::test]
async fn retrieve_scoped_returns_none_for_scope_mismatch() {
    let manager = create_test_manager();
    let id = CredentialId::new();
    let data = create_test_data("secret");
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();
    let ctx_a = CredentialContext::new("user").with_scope(ScopeLevel::Project(project_a));
    manager
        .store(&id, data, CredentialMetadata::new(), &ctx_a)
        .await
        .unwrap();

    let ctx_b = CredentialContext::new("user").with_scope(ScopeLevel::Project(project_b));
    let result = manager.retrieve_scoped(&id, &ctx_b).await.unwrap();

    assert!(
        result.is_none(),
        "retrieve_scoped must return None when scopes don't match"
    );
}

/// retrieve_scoped returns credential for exact scope match
#[tokio::test]
async fn retrieve_scoped_returns_credential_for_exact_match() {
    let manager = create_test_manager();
    let id = CredentialId::new();
    let data = create_test_data("secret");
    let project_id = ProjectId::new();
    let ctx = CredentialContext::new("user").with_scope(ScopeLevel::Project(project_id));
    manager
        .store(&id, data.clone(), CredentialMetadata::new(), &ctx)
        .await
        .unwrap();

    let result = manager.retrieve_scoped(&id, &ctx).await.unwrap();

    assert!(
        result.is_some(),
        "retrieve_scoped must return credential for exact scope match"
    );
}

/// retrieve_scoped returns credential when parent scope accesses child credential
#[tokio::test]
async fn retrieve_scoped_parent_can_access_child() {
    let manager = create_test_manager();
    let id = CredentialId::new();
    let data = create_test_data("secret");
    let project_id = ProjectId::new();
    let org_id = OrganizationId::new();
    let child_ctx =
        CredentialContext::new("user").with_scope(ScopeLevel::Project(project_id));
    manager
        .store(&id, data, CredentialMetadata::new(), &child_ctx)
        .await
        .unwrap();

    let parent_ctx =
        CredentialContext::new("user").with_scope(ScopeLevel::Organization(org_id));
    let result = manager.retrieve_scoped(&id, &parent_ctx).await.unwrap();

    assert!(
        result.is_some(),
        "Parent scope (Organization) must access child credential (Project)"
    );
}

/// list_scoped requires scope — returns Err(ScopeRequired) when caller_scope is None
#[tokio::test]
async fn list_scoped_requires_scope() {
    let manager = create_test_manager();
    let ctx_no_scope = CredentialContext::new("user");
    let result = manager.list_scoped(&ctx_no_scope).await;

    assert!(
        matches!(result, Err(ManagerError::ScopeRequired { .. })),
        "list_scoped must return ScopeRequired when context has no scope"
    );
}

/// list_scoped filters credentials by scope
#[tokio::test]
async fn list_scoped_filters_by_scope() {
    let manager = create_test_manager();
    let data = create_test_data("secret");
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();

    let ctx_a = CredentialContext::new("user").with_scope(ScopeLevel::Project(project_a));
    let id_a = CredentialId::new();
    manager
        .store(&id_a, data.clone(), CredentialMetadata::new(), &ctx_a)
        .await
        .unwrap();

    let ctx_b = CredentialContext::new("user").with_scope(ScopeLevel::Project(project_b));
    let id_b = CredentialId::new();
    manager
        .store(&id_b, data, CredentialMetadata::new(), &ctx_b)
        .await
        .unwrap();

    let ids_a = manager.list_scoped(&ctx_a).await.unwrap();
    assert_eq!(ids_a.len(), 1, "project_a should see only its credential");
    assert!(ids_a.contains(&id_a));

    let ids_b = manager.list_scoped(&ctx_b).await.unwrap();
    assert_eq!(ids_b.len(), 1, "project_b should see only its credential");
    assert!(ids_b.contains(&id_b));

    let org_id = OrganizationId::new();
    let parent_ctx =
        CredentialContext::new("user").with_scope(ScopeLevel::Organization(org_id));
    let ids_parent = manager.list_scoped(&parent_ctx).await.unwrap();
    assert_eq!(
        ids_parent.len(),
        2,
        "Organization scope should see both project credentials"
    );
}

/// retrieve does NOT enforce scope — documents expected behavior
#[tokio::test]
async fn retrieve_does_not_enforce_scope() {
    let manager = create_test_manager();
    let id = CredentialId::new();
    let data = create_test_data("secret");
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();
    let ctx_a = CredentialContext::new("user").with_scope(ScopeLevel::Project(project_a));
    manager
        .store(&id, data, CredentialMetadata::new(), &ctx_a)
        .await
        .unwrap();

    let ctx_b = CredentialContext::new("user").with_scope(ScopeLevel::Project(project_b));
    let result = manager.retrieve(&id, &ctx_b).await.unwrap();

    assert!(
        result.is_some(),
        "retrieve() does not enforce scope — use retrieve_scoped() for isolation"
    );
}
