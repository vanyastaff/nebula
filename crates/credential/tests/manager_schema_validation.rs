//! Integration tests for `CredentialManager::store_validated()`.
//!
//! Verifies that credential values are validated against the
//! `ParameterCollection` schema in `CredentialDescription` before storage.

use nebula_credential::core::{
    CredentialContext, CredentialDescription, CredentialId, CredentialMetadata, ManagerError,
};
use nebula_credential::prelude::*;
use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{NumberParameter, SecretParameter, TextParameter};
use nebula_parameter::validation::ValidationRule;
use nebula_parameter::values::ParameterValues;
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
    let mut client_id = TextParameter::new("client_id", "Client ID");
    client_id.metadata.required = true;
    client_id.validation.push(ValidationRule::min_length(5));

    let mut client_secret = SecretParameter::new("client_secret", "Client Secret");
    client_secret.metadata.required = true;

    let properties = ParameterCollection::new()
        .with(ParameterDef::Text(client_id))
        .with(ParameterDef::Secret(client_secret));

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
    let id = CredentialId::new("gh-valid").unwrap();
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
    let id = CredentialId::new("gh-missing").unwrap();
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
async fn store_validated_fails_on_type_mismatch() {
    let manager = test_manager();
    let id = CredentialId::new("gh-type-mismatch").unwrap();
    let context = CredentialContext::new("user-1");

    // Schema expects a number for "port"
    let mut port = NumberParameter::new("port", "Port");
    port.metadata.required = true;

    let properties = ParameterCollection::new().with(ParameterDef::Number(port));

    let description = CredentialDescription::builder()
        .key("db_conn")
        .name("DB Connection")
        .description("Database connection")
        .properties(properties)
        .build()
        .unwrap();

    // Provide a string instead of a number
    let mut values = ParameterValues::new();
    values.set("port", json!("not-a-number"));

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

    assert!(result.is_err(), "should fail on type mismatch");
    match result.unwrap_err() {
        ManagerError::SchemaValidation { errors, .. } => {
            assert_eq!(errors.len(), 1);
            let msg = errors[0].to_string();
            assert!(
                msg.contains("port") && msg.contains("number"),
                "error should describe the type mismatch: {msg}"
            );
        }
        other => panic!("expected SchemaValidation, got: {other:?}"),
    }
}

#[tokio::test]
async fn store_validated_fails_on_validation_rule_violation() {
    let manager = test_manager();
    let description = github_description();
    let id = CredentialId::new("gh-short-id").unwrap();
    let context = CredentialContext::new("user-1");

    // client_id has min_length(5) -- provide a 3-char string
    let mut values = ParameterValues::new();
    values.set("client_id", json!("abc"));
    values.set("client_secret", json!("some-secret"));

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

    assert!(result.is_err(), "should fail on min_length violation");
    match result.unwrap_err() {
        ManagerError::SchemaValidation { errors, .. } => {
            assert_eq!(errors.len(), 1);
            let msg = errors[0].to_string();
            assert!(
                msg.contains("client_id"),
                "error should mention client_id: {msg}"
            );
        }
        other => panic!("expected SchemaValidation, got: {other:?}"),
    }
}

#[tokio::test]
async fn store_validated_collects_multiple_errors() {
    let manager = test_manager();
    let description = github_description();
    let id = CredentialId::new("gh-multi-err").unwrap();
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
    let id = CredentialId::new("no-validation").unwrap();
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
