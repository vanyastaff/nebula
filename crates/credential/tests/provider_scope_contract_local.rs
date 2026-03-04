//! Contract test: storage providers are scope-agnostic; scope isolation is manager-level.

use nebula_core::{OrganizationId, ScopeLevel};
use nebula_credential::core::{CredentialContext, CredentialId, CredentialMetadata};
use nebula_credential::providers::{LocalStorageConfig, LocalStorageProvider};
use nebula_credential::traits::StorageProvider;
use nebula_credential::utils::{EncryptionKey, encrypt};
use tempfile::TempDir;

#[tokio::test]
async fn local_provider_does_not_enforce_scope() {
    let temp_dir = TempDir::new().expect("temp dir");
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config).expect("provider");

    let id = CredentialId::new();
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").expect("encrypt");
    let metadata = CredentialMetadata::default();

    let ctx_a = CredentialContext::new("user_a")
        .with_scope(ScopeLevel::Organization(OrganizationId::new()));
    let ctx_b = CredentialContext::new("user_b")
        .with_scope(ScopeLevel::Organization(OrganizationId::new()));

    provider
        .store(&id, data.clone(), metadata, &ctx_a)
        .await
        .expect("store");

    let (retrieved_data, _) = provider.retrieve(&id, &ctx_b).await.expect("retrieve");
    assert_eq!(retrieved_data.ciphertext, data.ciphertext);
}
