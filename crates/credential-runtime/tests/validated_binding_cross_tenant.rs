//! Probe: a workflow `slot_bindings` reference that points to a
//! credential in a different tenant must fail validation with
//! [`ValidatedCredentialBindingError::ScopeMismatch`]. Closes the
//! confused-deputy non-goal from the ADR-0052 cascade.

use nebula_credential_runtime::test_support::in_memory_service;
use nebula_credential_runtime::{TenantScope, ValidatedCredentialBindingError};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_binding_rejected() {
    let service = in_memory_service();

    let scope_a = TenantScope::new("org", "ws-a");
    let scope_b = TenantScope::new("org", "ws-b");

    // Create a credential under scope A.
    service
        .create(
            &scope_a,
            "bearer_token",
            json!({ "token": "tenant-a-key" }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect("create succeeds for tenant A");

    // Retrieve the created credential id via list.
    let ids_a = service.list(&scope_a).await.expect("list tenant A");
    assert_eq!(
        ids_a.len(),
        1,
        "expected exactly one credential under scope A"
    );
    let id = &ids_a[0];

    // Tenant B tries to bind to tenant A's credential — must fail with ScopeMismatch.
    let err = service
        .validate_credential_binding(&scope_b, id)
        .await
        .expect_err("scope B must not bind to scope A's credential");

    assert!(
        matches!(err, ValidatedCredentialBindingError::ScopeMismatch { .. }),
        "expected ScopeMismatch, got {err:?}"
    );

    // Sanity: tenant A CAN bind its own credential.
    let binding = service
        .validate_credential_binding(&scope_a, id)
        .await
        .expect("scope A must be able to bind its own credential");

    assert_eq!(binding.credential_id(), id.as_str());
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_credential_is_not_found() {
    let service = in_memory_service();
    let scope = TenantScope::new("org", "ws-a");

    let err = service
        .validate_credential_binding(&scope, "cred_01NONEXISTENT0000000000000")
        .await
        .expect_err("absent credential must return NotFound");

    assert!(
        matches!(err, ValidatedCredentialBindingError::NotFound { .. }),
        "expected NotFound for absent id, got {err:?}"
    );
}
