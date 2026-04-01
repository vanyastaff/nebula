//! Integration test: resolve → snapshot → typed access.
//!
//! Validates the full credential resolution → typed access pipeline:
//! store → resolve → handle → snapshot → project.

use std::sync::Arc;

use nebula_credential::credentials::ApiKeyCredential;
use nebula_credential::scheme::{BearerToken, DatabaseAuth};
use nebula_credential::store::{PutMode, StoredCredential};
use nebula_credential::{
    Credential, CredentialMetadata, CredentialResolver, CredentialSnapshot, CredentialStore,
    InMemoryStore, SnapshotError,
};

#[tokio::test]
async fn resolve_to_typed_snapshot() {
    let store = Arc::new(InMemoryStore::new());

    // Store a bearer token credential.
    // Note: SecretString serializes as "[REDACTED]", so we construct raw JSON
    // directly — the real store holds encrypted raw values.
    let data = br#"{"token":"test-key"}"#.to_vec();
    let cred = StoredCredential {
        id: "test-cred".into(),
        credential_key: "api_key".into(),
        data,
        state_kind: "bearer".into(),
        state_version: 1,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        expires_at: None,
        metadata: Default::default(),
    };
    store.put(cred, PutMode::CreateOnly).await.unwrap();

    // Resolve via the resolver
    let resolver = CredentialResolver::new(store);
    let handle = resolver
        .resolve::<ApiKeyCredential>("test-cred")
        .await
        .unwrap();

    // Build snapshot (simulating what the runtime does)
    let snapshot = CredentialSnapshot::new(
        ApiKeyCredential::KEY,
        CredentialMetadata::new(),
        (*handle.snapshot()).clone(),
    );

    // Typed access works
    assert!(snapshot.is::<BearerToken>());
    let token = snapshot.project::<BearerToken>().unwrap();
    token.expose().expose_secret(|s| assert_eq!(s, "test-key"));

    // Wrong type fails cleanly
    assert!(!snapshot.is::<DatabaseAuth>());
    let err = snapshot.project::<DatabaseAuth>().unwrap_err();
    assert!(matches!(
        err,
        SnapshotError::SchemeMismatch {
            expected: "database",
            ..
        }
    ));

    // Clone works
    let cloned = snapshot.clone();
    let cloned_token = cloned.project::<BearerToken>().unwrap();
    cloned_token
        .expose()
        .expose_secret(|s| assert_eq!(s, "test-key"));
}
