//! Integration test: resolve -> snapshot -> typed access.
//!
//! Validates the full credential resolution -> typed access pipeline:
//! store -> resolve -> handle -> snapshot -> project.

use std::sync::Arc;

use nebula_credential::{
    Credential, CredentialRecord, CredentialResolver, CredentialSnapshot, CredentialStore,
    InMemoryStore, SnapshotError,
    credentials::ApiKeyCredential,
    scheme::{ConnectionUri, SecretToken},
    store::{PutMode, StoredCredential},
};

#[tokio::test]
async fn resolve_to_typed_snapshot() {
    let store = Arc::new(InMemoryStore::new());

    // Store a secret token credential.
    // Note: SecretString serializes as "[REDACTED]", so we construct raw JSON
    // directly -- the real store holds encrypted raw values.
    let data = br#"{"token":"test-key"}"#.to_vec();
    let cred = StoredCredential {
        id: "test-cred".into(),
        credential_key: "api_key".into(),
        data,
        state_kind: "secret_token".into(),
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
        CredentialRecord::new(),
        (*handle.snapshot()).clone(),
    );

    // Typed access works
    assert!(snapshot.is::<SecretToken>());
    let token = snapshot.project::<SecretToken>().unwrap();
    token.token().expose_secret(|s| assert_eq!(s, "test-key"));

    // Wrong type fails cleanly
    assert!(!snapshot.is::<ConnectionUri>());
    let err = snapshot.project::<ConnectionUri>().unwrap_err();
    match &err {
        SnapshotError::SchemeMismatch { expected, .. } => {
            assert_eq!(expected, "ConnectionUri");
        },
        _ => panic!("unexpected error variant"),
    }

    // Clone works
    let cloned = snapshot.clone();
    let cloned_token = cloned.project::<SecretToken>().unwrap();
    cloned_token
        .token()
        .expose_secret(|s| assert_eq!(s, "test-key"));
}
