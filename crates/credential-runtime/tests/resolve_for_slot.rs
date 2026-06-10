//! Probe: `resolve_for_slot` produces a typed `CredentialGuard` for a
//! validated binding. End-to-end: create → validate_binding → resolve
//! → guard ready for action consumption.
//!
//! Uses `BearerTokenCredential` (from `nebula-credential-builtin`) because
//! `in_memory_service()` registers the three first-party builtins:
//! `bearer_token`, `shared_key`, and `signing_key`. `ApiKeyCredential` is in
//! `nebula-credential` (a different crate) and is not wired into the
//! `in_memory_service()` fixture.

use nebula_credential::Credential;
use nebula_credential::scheme::SecretToken;
use nebula_credential_builtin::BearerTokenCredential;
use nebula_credential_runtime::test_support::in_memory_service;
use nebula_credential_runtime::{CredentialServiceError, TenantScope};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn resolve_for_slot_produces_guard() {
    let service = in_memory_service();
    let scope = TenantScope::new("org", "ws");

    // Step 1: create a credential.
    service
        .create(
            &scope,
            BearerTokenCredential::KEY,
            json!({ "token": "test-bearer-abc123" }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect("create succeeds");

    let heads = service.list(&scope).await.expect("list succeeds");
    assert_eq!(heads.len(), 1, "expected exactly one credential");
    let id = &heads[0].id;

    // Step 2: validate the binding.
    let binding = service
        .validate_credential_binding(&scope, id)
        .await
        .expect("validate succeeds");

    assert_eq!(binding.credential_id(), id.as_str());

    // Step 3: resolve via the binding → typed guard.
    let cancel = CancellationToken::new();
    let _guard: nebula_credential::CredentialGuard<SecretToken> = service
        .resolve_for_slot::<BearerTokenCredential>(&scope, &binding, cancel)
        .await
        .expect("resolve_for_slot succeeds");

    // The guard's Drop will zeroize the underlying scheme.
    // We don't assert on the contents (they're sensitive) — this test
    // asserts the entire typed chain returned Ok.
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_for_slot_scope_violation_rejected() {
    let service = in_memory_service();
    let scope_a = TenantScope::new("org", "ws-a");
    let scope_b = TenantScope::new("org", "ws-b");

    // Create a credential under scope A.
    service
        .create(
            &scope_a,
            BearerTokenCredential::KEY,
            json!({ "token": "token-for-a" }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect("create succeeds");

    let heads_a = service.list(&scope_a).await.expect("list scope A");
    let id = &heads_a[0].id;

    // Validate the binding for scope A.
    let binding_a = service
        .validate_credential_binding(&scope_a, id)
        .await
        .expect("validate for scope A succeeds");

    // Attempt to resolve the scope-A binding via scope B — defence-in-depth
    // fingerprint check must reject this.
    let cancel = CancellationToken::new();
    let err = service
        .resolve_for_slot::<BearerTokenCredential>(&scope_b, &binding_a, cancel)
        .await
        .expect_err("cross-scope resolve must be rejected");

    assert!(
        matches!(err, CredentialServiceError::ScopeViolation { .. }),
        "expected ScopeViolation, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_for_slot_cancellation_returns_cancelled() {
    let service = in_memory_service();
    let scope = TenantScope::new("org", "ws");

    service
        .create(
            &scope,
            BearerTokenCredential::KEY,
            json!({ "token": "token-cancel-test" }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect("create succeeds");

    let heads = service.list(&scope).await.expect("list succeeds");
    let id = &heads[0].id;

    let binding = service
        .validate_credential_binding(&scope, id)
        .await
        .expect("validate succeeds");

    // Pre-cancel the token before the call.
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = service
        .resolve_for_slot::<BearerTokenCredential>(&scope, &binding, cancel)
        .await
        .expect_err("pre-cancelled token must return Cancelled");

    assert!(
        matches!(err, CredentialServiceError::Cancelled),
        "expected Cancelled, got {err:?}"
    );
}
