//! Integration tests for CredentialManager::create().
//!
//! Verifies protocol registry dispatch, static protocol (api_key, basic_auth),
//! and storage of completed credentials.

use nebula_credential::prelude::*;
use nebula_parameter::values::ParameterValues;
use std::sync::Arc;

/// Helper to create a test manager with encryption key (required for create storage)
async fn create_test_manager() -> CredentialManager {
    let storage = MockStorageProvider::new();
    let key = Arc::new(EncryptionKey::from_bytes([0u8; 32]));
    CredentialManager::builder()
        .storage(Arc::new(storage))
        .encryption_key(key)
        .build()
}

#[tokio::test]
async fn test_create_api_key_complete() {
    let manager = create_test_manager().await;
    let mut values = ParameterValues::new();
    values.set("server", serde_json::json!("https://api.example.com"));
    values.set("token", serde_json::json!("secret-token-123"));
    let context = CredentialContext::new("user-1");

    let result = manager.create("api_key", &values, &context).await.unwrap();

    match &result {
        CreateResult::Complete {
            credential_id,
            type_id,
        } => {
            assert_eq!(type_id, "api_key");
            assert!(!credential_id.is_nil());

            // Verify credential was stored
            let (data, _meta) = manager
                .retrieve(credential_id, &context)
                .await
                .unwrap()
                .unwrap();
            assert!(!data.ciphertext.is_empty());
        }
        _ => panic!("expected Complete, got {:?}", result),
    }
}

#[tokio::test]
async fn test_create_basic_auth_complete() {
    let manager = create_test_manager().await;
    let mut values = ParameterValues::new();
    values.set("username", serde_json::json!("alice"));
    values.set("password", serde_json::json!("s3cr3t"));
    let context = CredentialContext::new("user-1");

    let result = manager
        .create("basic_auth", &values, &context)
        .await
        .unwrap();

    match &result {
        CreateResult::Complete {
            credential_id,
            type_id,
        } => {
            assert_eq!(type_id, "basic_auth");
            assert!(!credential_id.is_nil());
        }
        _ => panic!("expected Complete, got {:?}", result),
    }
}

#[tokio::test]
async fn test_create_unknown_type_returns_error() {
    let manager = create_test_manager().await;
    let values = ParameterValues::new();
    let context = CredentialContext::new("user-1");

    let result = manager.create("unknown_type", &values, &context).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown credential type"));
}

#[tokio::test]
async fn test_create_without_encryption_key_returns_error() {
    let storage = MockStorageProvider::new();
    let manager = CredentialManager::builder()
        .storage(Arc::new(storage))
        .build();
    let mut values = ParameterValues::new();
    values.set("server", serde_json::json!("https://api.example.com"));
    values.set("token", serde_json::json!("secret"));
    let context = CredentialContext::new("user-1");

    let result = manager.create("api_key", &values, &context).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("encryption_key"));
}
