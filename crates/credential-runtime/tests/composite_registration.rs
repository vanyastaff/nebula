//! Probe: `register_credential_complete` is atomic on conflict — a
//! second registration of the same type fails without leaving partial
//! state in any of the three registries.

use nebula_credential::CredentialRegistry;
use nebula_credential_builtin::BearerTokenCredential;
use nebula_credential_runtime::{
    CredentialDispatch, RegistrationError, register_credential_complete,
};
use nebula_engine::credential::StateProjectionRegistry;

#[test]
fn composite_registration_atomic_on_duplicate_key() {
    let mut cred = CredentialRegistry::new();
    let mut proj = StateProjectionRegistry::new();
    let mut disp = CredentialDispatch::new();

    register_credential_complete(
        &mut cred,
        &mut proj,
        &mut disp,
        BearerTokenCredential,
        "nebula-credential-builtin",
    )
    .expect("first registration succeeds");

    let cred_count_before = cred.iter_keys().count();
    let proj_count_before = proj.iter_keys().count();
    let disp_count_before = disp.iter_keys().count();

    let err = register_credential_complete(
        &mut cred,
        &mut proj,
        &mut disp,
        BearerTokenCredential,
        "nebula-credential-builtin",
    )
    .expect_err("duplicate registration must fail");

    assert!(
        matches!(err, RegistrationError::DuplicateKey { .. }),
        "expected DuplicateKey, got {err:?}",
    );

    // No partial state left behind.
    assert_eq!(cred.iter_keys().count(), cred_count_before);
    assert_eq!(proj.iter_keys().count(), proj_count_before);
    assert_eq!(disp.iter_keys().count(), disp_count_before);
}
