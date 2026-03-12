//! Integration tests for `CredentialManager::store_validated()`.
//!
//! Verifies that credential values are validated against the `Schema` in
//! `CredentialDescription` before storage.

use nebula_credential::core::{
    CredentialContext, CredentialDescription, CredentialId, CredentialMetadata, ManagerError,
};
use nebula_credential::prelude::*;
use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Field, Schema};
use serde_json::json;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_manager() -> CredentialManager {
    CredentialManager::builder()
        .storage(Arc::new(MockStorageProvider::new()))
        .build()
}

fn github_description() -> CredentialDescription {
    let properties = Schema::new()
        .field(Field::text("client_id").with_label("Client ID").required())
        .field(
            Field::text("client_secret")
                .with_label("Client Secret")
                .required()
                .secret(),
        );

    CredentialDescription::builder()
        .key("github_oauth2")
        .name("GitHub OAuth2")
        .description("OAuth2 authentication for GitHub API")
        .properties(properties)
        .build()
        .expect("test description should build")
}

fn test_encrypted_data() -> EncryptedData {
    let key = EncryptionKey::from_bytes([0u8; 32]);
    encrypt(&key, b"encrypted-blob").expect("encryption should succeed")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn store_validated_succeeds_with_valid_values() {
    let manager = test_manager();
    let description = github_description();
    let id = CredentialId::new();
    let context = CredentialContext::new("user-1");

    let mut values = ParameterValues::new();
    values.set("client_id", json!("my-client-id"));
    values.set("client_secret", json!("super-secret"));

    let result = manager
        .store_validated(
            &id,
            &description,
            &values,
            test_encrypted_data(),
            CredentialMetadata::new(),
            &context,
        )
        .await;

    assert!(result.is_ok(), "store_validated should succeed: {result:?}");

    // Verify the credential was actually persisted
    let retrieved = manager.retrieve(&id, &context).await.unwrap();
    assert!(retrieved.is_some(), "credential should be retrievable");
}

#[tokio::test]
async fn store_validated_fails_on_missing_required_field() {
    let manager = test_manager();
    let description = github_description();
    let id = CredentialId::new();
    let context = CredentialContext::new("user-1");

    // Only provide client_id, omit required client_secret
    let mut values = ParameterValues::new();
    values.set("client_id", json!("my-client-id"));

    let result = manager
        .store_validated(
            &id,
            &description,
            &values,
            test_encrypted_data(),
            CredentialMetadata::new(),
            &context,
        )
        .await;

    assert!(result.is_err(), "should fail with missing required field");
    let err = result.unwrap_err();
    match &err {
        ManagerError::SchemaValidation {
            credential_type,
            errors,
        } => {
            assert_eq!(credential_type, "github_oauth2");
            assert_eq!(errors.len(), 1, "should have exactly one error");
            assert!(
                errors[0].to_string().contains("client_secret"),
                "error should mention client_secret: {}",
                errors[0]
            );
        }
        other => panic!("expected SchemaValidation, got: {other:?}"),
    }

    // Verify the credential was NOT stored
    let retrieved = manager.retrieve(&id, &context).await.unwrap();
    assert!(
        retrieved.is_none(),
        "credential should not exist after validation failure"
    );
}

#[tokio::test]
async fn store_validated_collects_multiple_errors() {
    let manager = test_manager();
    let description = github_description();
    let id = CredentialId::new();
    let context = CredentialContext::new("user-1");

    // Both required fields missing
    let values = ParameterValues::new();

    let result = manager
        .store_validated(
            &id,
            &description,
            &values,
            test_encrypted_data(),
            CredentialMetadata::new(),
            &context,
        )
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ManagerError::SchemaValidation { errors, .. } => {
            assert_eq!(
                errors.len(),
                2,
                "should report errors for both missing required fields"
            );
        }
        other => panic!("expected SchemaValidation, got: {other:?}"),
    }
}

#[tokio::test]
async fn existing_store_still_works_without_validation() {
    // Backward compatibility: the original store() must keep working
    let manager = test_manager();
    let id = CredentialId::new();
    let context = CredentialContext::new("user-1");

    let result = manager
        .store(
            &id,
            test_encrypted_data(),
            CredentialMetadata::new(),
            &context,
        )
        .await;

    assert!(
        result.is_ok(),
        "existing store() should work unchanged: {result:?}"
    );

    let retrieved = manager.retrieve(&id, &context).await.unwrap();
    assert!(retrieved.is_some());
}
