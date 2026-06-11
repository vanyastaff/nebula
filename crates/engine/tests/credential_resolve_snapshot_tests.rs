//! Integration test: resolve -> snapshot -> typed access (engine-owned resolver).

use std::sync::Arc;

use nebula_credential::{
    Credential, CredentialRecord, CredentialSnapshot, CredentialStore, SnapshotError,
    credentials::ApiKeyCredential,
    scheme::{ConnectionUri, SecretToken},
    store::{PutMode, StoredCredential},
};
use nebula_storage::credential::SqliteCredentialStore;

#[tokio::test]
async fn resolve_to_typed_snapshot() {
    let store = Arc::new(
        SqliteCredentialStore::connect_memory()
            .await
            .expect("in-memory SQLite store"),
    );

    let data = br#"{"token":"test-key"}"#.to_vec();
    let cred = StoredCredential {
        id: "test-cred".into(),
        name: None,
        credential_key: "api_key".into(),
        data,
        state_kind: "secret_token".into(),
        state_version: 1,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        expires_at: None,
        reauth_required: false,
        metadata: Default::default(),
    };
    store.put(cred, PutMode::CreateOnly).await.unwrap();

    let resolver = nebula_engine::credential::CredentialResolver::new(store).unwrap();
    let handle = resolver
        .resolve::<ApiKeyCredential>("test-cred")
        .await
        .unwrap();

    let snapshot = CredentialSnapshot::new(
        ApiKeyCredential::KEY,
        CredentialRecord::new(),
        (*handle.snapshot()).clone(),
    );

    assert!(snapshot.is::<SecretToken>());
    let token = snapshot.project::<SecretToken>().unwrap();
    assert_eq!(token.token().expose_secret(), "test-key");

    assert!(!snapshot.is::<ConnectionUri>());
    let err = snapshot.project::<ConnectionUri>().unwrap_err();
    match &err {
        SnapshotError::SchemeMismatch { expected, .. } => {
            assert_eq!(expected, "ConnectionUri");
        },
        _ => panic!("unexpected error variant"),
    }
}
