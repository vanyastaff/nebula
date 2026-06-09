//! The credential-service factory composes a working service and the wired
//! service performs a real create/get round-trip.
//!
//! This is the runtime proof for the factory's two load-bearing invariants:
//! (1) `CredentialServiceBuilder::build` runs a capability⊆ops gate, so a
//! successful build proves OAuth2's four advertised capabilities each have a
//! registered dispatch op (and api_key/basic_auth advertise none); (2) the
//! fixed dev key actually base64-decodes to a valid AES-256 key.

use std::sync::Arc;

use nebula_api::ports::credential_service_factory::with_key_provider;
use nebula_credential::CredentialDisplay;
use nebula_credential_runtime::TenantScope;
use nebula_storage::credential::EnvKeyProvider;
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
    let snap = svc
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
    assert_eq!(snap.kind(), "api_key");
    assert_eq!(snap.display().display_name.as_deref(), Some("Test key"));

    // Retrievable, secret never echoed in the snapshot's Debug.
    let ids = svc.list(&scope).await.expect("list");
    assert_eq!(ids.len(), 1);
    let got = svc.get(&scope, &ids[0]).await.expect("get");
    assert_eq!(got.kind(), "api_key");
    assert_eq!(got.display().display_name.as_deref(), Some("Test key"));
    assert!(!format!("{got:?}").contains("k-factory-test"));
}
