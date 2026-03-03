//! Integration tests for retrieve_scoped() and list_scoped() methods
//!
//! Uses ScopeLevel from nebula-core for platform consistency.

use nebula_credential::core::ManagerError;
use nebula_credential::prelude::*;
use nebula_core::{OrganizationId, ProjectId, ScopeLevel, WorkflowId};
use std::sync::Arc;

async fn create_test_manager() -> CredentialManager {
    CredentialManager::builder()
        .storage(Arc::new(MockStorageProvider::new()))
        .build()
}

fn create_test_data(value: &str) -> EncryptedData {
    let key = EncryptionKey::from_bytes([0u8; 32]);
    encrypt(&key, value.as_bytes()).unwrap()
}

#[tokio::test]
async fn test_retrieve_scoped_exact_match() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("password");
    let metadata = CredentialMetadata::new();
    let project_id = ProjectId::new();

    let context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_id));

    manager
        .store(&id, data.clone(), metadata, &context)
        .await
        .unwrap();

    let result = manager.retrieve_scoped(&id, &context).await.unwrap();
    assert!(result.is_some());

    let (_, retrieved_metadata) = result.unwrap();
    assert!(matches!(
        retrieved_metadata.owner_scope.as_ref(),
        Some(ScopeLevel::Project(id)) if *id == project_id
    ));
}

#[tokio::test]
async fn test_retrieve_scoped_hierarchical_match() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("api-key");
    let metadata = CredentialMetadata::new();
    let project_id = ProjectId::new();
    let org_id = OrganizationId::new();

    let project_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_id));

    manager
        .store(&id, data, metadata, &project_context)
        .await
        .unwrap();

    let org_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Organization(org_id));

    let result = manager.retrieve_scoped(&id, &org_context).await.unwrap();
    assert!(result.is_some(), "Organization scope should access Project credential");
}

#[tokio::test]
async fn test_retrieve_scoped_mismatch() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("secret");
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

    let result = manager.retrieve_scoped(&id, &team_b_context).await.unwrap();
    assert!(result.is_none(), "Cross-scope access should be denied");
}

#[tokio::test]
async fn test_retrieve_scoped_no_context_scope() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let context = CredentialContext::new("user-1");

    let result = manager.retrieve_scoped(&id, &context).await;
    assert!(result.is_err(), "Should require scope in context");

    match result.unwrap_err() {
        ManagerError::ScopeRequired { operation } => assert_eq!(operation, "retrieve_scoped"),
        e => panic!("Expected ScopeRequired error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_retrieve_scoped_unscoped_credential() {
    let manager = create_test_manager().await;
    let id = CredentialId::new();
    let data = create_test_data("legacy-secret");
    let metadata = CredentialMetadata::new();

    let unscoped_context = CredentialContext::new("user-1");
    manager
        .store(&id, data, metadata, &unscoped_context)
        .await
        .unwrap();

    let org_id = OrganizationId::new();
    let scoped_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Organization(org_id));

    let result = manager.retrieve_scoped(&id, &scoped_context).await.unwrap();
    assert!(
        result.is_none(),
        "Unscoped credentials not accessible via retrieve_scoped"
    );
}

#[tokio::test]
async fn test_list_scoped_exact_match() {
    let manager = create_test_manager().await;
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();

    let scope_a_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_a));
    let scope_b_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_b));

    let cred_a1 = CredentialId::new();
    let cred_a2 = CredentialId::new();
    let cred_b1 = CredentialId::new();

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

    let a_creds = manager.list_scoped(&scope_a_context).await.unwrap();
    assert_eq!(a_creds.len(), 2);
    assert!(a_creds.contains(&cred_a1));
    assert!(a_creds.contains(&cred_a2));

    let b_creds = manager.list_scoped(&scope_b_context).await.unwrap();
    assert_eq!(b_creds.len(), 1);
    assert!(b_creds.contains(&cred_b1));
}

#[tokio::test]
async fn test_list_scoped_hierarchical() {
    let manager = create_test_manager().await;
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();
    let project_id = ProjectId::new();
    let wf_id = WorkflowId::new();
    let org_id = OrganizationId::new();

    let project_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Project(project_id));
    let workflow_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Workflow(wf_id));
    let org_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Organization(org_id));

    let project_cred = CredentialId::new();
    let workflow_cred = CredentialId::new();
    let org_cred = CredentialId::new();

    manager
        .store(
            &project_cred,
            data.clone(),
            metadata.clone(),
            &project_context,
        )
        .await
        .unwrap();
    manager
        .store(
            &workflow_cred,
            data.clone(),
            metadata.clone(),
            &workflow_context,
        )
        .await
        .unwrap();
    manager
        .store(&org_cred, data.clone(), metadata.clone(), &org_context)
        .await
        .unwrap();

    let org_creds = manager.list_scoped(&org_context).await.unwrap();
    assert_eq!(org_creds.len(), 3, "Organization sees all child credentials");
    assert!(org_creds.contains(&project_cred));
    assert!(org_creds.contains(&workflow_cred));
    assert!(org_creds.contains(&org_cred));

    let project_creds = manager.list_scoped(&project_context).await.unwrap();
    assert_eq!(
        project_creds.len(),
        2,
        "Project sees its credential and child Workflow credential"
    );
    assert!(project_creds.contains(&project_cred));
    assert!(project_creds.contains(&workflow_cred));
}

#[tokio::test]
async fn test_list_scoped_no_context_scope() {
    let manager = create_test_manager().await;
    let context = CredentialContext::new("user-1");

    let result = manager.list_scoped(&context).await;
    assert!(result.is_err(), "Should require scope in context");

    match result.unwrap_err() {
        ManagerError::ScopeRequired { operation } => assert_eq!(operation, "list_scoped"),
        e => panic!("Expected ScopeRequired error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_list_scoped_excludes_unscoped() {
    let manager = create_test_manager().await;
    let data = create_test_data("secret");
    let metadata = CredentialMetadata::new();
    let org_id = OrganizationId::new();

    let scoped_context =
        CredentialContext::new("user-1").with_scope(ScopeLevel::Organization(org_id));
    let unscoped_context = CredentialContext::new("user-1");

    let scoped_cred = CredentialId::new();
    let unscoped_cred = CredentialId::new();

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

    let scoped_list = manager.list_scoped(&scoped_context).await.unwrap();
    assert_eq!(scoped_list.len(), 1);
    assert!(scoped_list.contains(&scoped_cred));
    assert!(!scoped_list.contains(&unscoped_cred));
}
