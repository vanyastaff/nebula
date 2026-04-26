//! Gate 2 (N7 mitigation) — `StateProjectionRegistry::register` rejects
//! duplicate `<C::State as CredentialState>::KIND` on second registration.
//!
//! Verifies:
//! 1. First registration succeeds (`Ok(())`).
//! 2. Second registration of the same credential type returns
//!    [`StateProjectionError::DuplicateKind`] with the colliding `kind`.
//! 3. Registry state is unchanged after the failure — original handler is not overwritten (len
//!    remains 1, `contains(kind)` still true).
//!
//! Active-dev policy per Tech Spec §15.12.2: reject-second-registration.
//! Silent `HashMap::insert` overwrite (prior behavior) would hide
//! namespace collisions, including supply-chain plugin replacement.
//!
//! The companion KEY-keyed [`CredentialRegistry`](nebula_credential::CredentialRegistry)
//! is exercised by `crates/credential/tests/runtime_duplicate_key_fatal.rs`
//! (Probe 5, Tech Spec §15.6). Both registries fail-closed on duplicate
//! registration; this probe covers the engine-side state-projection
//! lookup.

use nebula_credential::credentials::ApiKeyCredential;
use nebula_engine::{StateProjectionError, StateProjectionRegistry};

#[test]
fn first_registration_succeeds() {
    let mut registry = StateProjectionRegistry::new();
    assert!(registry.register::<ApiKeyCredential>().is_ok());
    assert_eq!(registry.len(), 1);
    assert!(registry.contains("secret_token"));
}

#[test]
fn duplicate_registration_returns_error_no_overwrite() {
    let mut registry = StateProjectionRegistry::new();

    // First registration succeeds.
    registry
        .register::<ApiKeyCredential>()
        .expect("first registration must succeed");
    assert_eq!(registry.len(), 1);

    // Second registration with same `<State as CredentialState>::KIND`
    // must fail with DuplicateKind — active-dev policy rejects silent
    // overwrite (N7 mitigation, Tech Spec §15.6 + §15.12.2).
    let err = registry
        .register::<ApiKeyCredential>()
        .expect_err("second registration must fail");

    match err {
        StateProjectionError::DuplicateKind { kind } => {
            assert_eq!(
                kind, "secret_token",
                "DuplicateKind must carry the colliding KIND verbatim"
            );
        },
        other => panic!("expected DuplicateKind, got {other:?}"),
    }

    // Registry state unchanged — first handler still present, not overwritten.
    assert_eq!(
        registry.len(),
        1,
        "duplicate registration must not increase handler count"
    );
    assert!(
        registry.contains("secret_token"),
        "original handler must still be registered after rejected duplicate"
    );
}

#[test]
fn duplicate_error_message_includes_policy_hint() {
    let mut registry = StateProjectionRegistry::new();
    registry.register::<ApiKeyCredential>().unwrap();
    let err = registry.register::<ApiKeyCredential>().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate state kind"),
        "error message must identify the failure class"
    );
    assert!(
        msg.contains("reject-second-registration"),
        "error message must state the active-dev policy"
    );
    assert!(
        msg.contains("secret_token"),
        "error message must include the colliding kind"
    );
}
