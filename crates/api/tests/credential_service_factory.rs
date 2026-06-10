//! The credential-service factory composes a working service and the wired
//! service performs a real create/get round-trip.
//!
//! This is the runtime proof for the factory's two load-bearing invariants:
//! (1) `CredentialServiceBuilder::build` runs a capability⊆ops gate, so a
//! successful build proves OAuth2's four advertised capabilities each have a
//! registered dispatch op (and api_key/basic_auth advertise none); (2) the
//! fixed dev key actually base64-decodes to a valid AES-256 key.

use std::sync::Arc;

use nebula_api::ports::credential_service_factory::{with_key_provider, with_store};
use nebula_credential::CredentialDisplay;
use nebula_credential_runtime::TenantScope;
use nebula_storage::credential::{EnvKeyProvider, SqliteCredentialStore};
use serde_json::json;

/// 32 `0x42` bytes, base64 — a valid AES-256 key fixture (mirrors the
/// factory's dev key). Not a secret: a fixed test constant.
const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

#[tokio::test]
async fn factory_builds_service_and_create_round_trips() {
    let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));
    // `build()` runs the capability⊆ops gate; success proves the registry's
    // advertised caps match the registered ops (OAuth2's four + none for the
    // static types).
    let svc = with_key_provider(key).expect("service composes (advertised caps match ops)");

    // A non-interactive create round-trips through the wired ops + display.
    let scope = TenantScope::new("org", "ws");
    let head = svc
        .create(
            &scope,
            "api_key",
            json!({ "api_key": "k-factory-test" }),
            CredentialDisplay {
                display_name: Some("Test key".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("api_key create succeeds");
    assert_eq!(head.credential_key, "api_key");
    assert_eq!(head.display.display_name.as_deref(), Some("Test key"));

    // Retrievable, secret never echoed in the head's Debug.
    let heads = svc.list(&scope).await.expect("list");
    assert_eq!(heads.len(), 1);
    assert_eq!(heads[0].id, head.id);
    let got = svc.get(&scope, &head.id).await.expect("get");
    assert_eq!(got.credential_key, "api_key");
    assert_eq!(got.display.display_name.as_deref(), Some("Test key"));
    assert!(!format!("{got:?}").contains("k-factory-test"));
}

#[tokio::test]
async fn factory_composes_over_durable_sqlite_store_and_round_trips() {
    // The production path composes the service over a durable SQLite backend
    // (here an ephemeral `:memory:` DB so the test needs no filesystem). Proves
    // `with_store` wires `SqliteCredentialStore` behind the full layer stack and
    // a create/get round-trips through the durable backend.
    let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));
    let store = SqliteCredentialStore::connect("sqlite::memory:")
        .await
        .expect("open + migrate in-memory SQLite");
    let svc = with_store(store, key).expect("service composes over SQLite backend");

    let scope = TenantScope::new("org", "ws");
    let head = svc
        .create(
            &scope,
            "api_key",
            json!({ "api_key": "k-sqlite-test" }),
            CredentialDisplay {
                display_name: Some("Durable key".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("api_key create succeeds against SQLite store");
    assert_eq!(head.credential_key, "api_key");

    let got = svc
        .get(&scope, &head.id)
        .await
        .expect("get from SQLite store");
    assert_eq!(got.display.display_name.as_deref(), Some("Durable key"));
    assert!(!format!("{got:?}").contains("k-sqlite-test"));
}
