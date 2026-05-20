//! Probe: every credential type registered into the credential KEY-registry
//! must also have:
//! - a state-projection entry in
//!   [`nebula_engine::credential::StateProjectionRegistry`] (so runtime
//!   dispatch can deserialize stored state)
//! - a capability-dispatch entry in
//!   [`nebula_credential_runtime::CredentialDispatch`] (so the service can
//!   dispatch refresh/test/revoke)
//!
//! Drift between these three is the silent-fatal vector identified in the
//! stabilize sweep design. A test that registers each first-party builtin
//! into all three and asserts agreement locks the invariant at CI time.
//!
//! Key-namespace note: `CredentialRegistry` and `CredentialDispatch` are
//! both keyed by `Credential::KEY` (e.g. `"bearer_token"`) — equality of
//! key sets is required. `StateProjectionRegistry` is keyed by
//! `<C::State as CredentialState>::KIND` (e.g. `"secret_token"`) — a
//! different namespace, so only entry COUNT is cross-checked against the
//! other two.

use std::collections::HashSet;

use nebula_credential::CredentialRegistry;
use nebula_credential_builtin::{
    BearerTokenCredential, SharedKeyCredential, SigningKeyCredential, register_builtins,
};
use nebula_credential_runtime::CredentialDispatch;
use nebula_engine::credential::StateProjectionRegistry;

#[test]
fn all_builtin_credentials_present_in_three_registries() {
    // --- CredentialRegistry (KEY-keyed) ---
    let mut cred = CredentialRegistry::new();
    register_builtins(&mut cred).expect("register builtins into CredentialRegistry");

    // --- StateProjectionRegistry (state-KIND-keyed) ---
    let mut proj = StateProjectionRegistry::new();
    proj.register::<BearerTokenCredential>()
        .expect("proj: bearer_token");
    proj.register::<SharedKeyCredential>()
        .expect("proj: shared_key");
    proj.register::<SigningKeyCredential>()
        .expect("proj: signing_key");

    // --- CredentialDispatch (KEY-keyed) ---
    let mut disp = CredentialDispatch::new();
    disp.register::<BearerTokenCredential>()
        .expect("disp: bearer_token");
    disp.register::<SharedKeyCredential>()
        .expect("disp: shared_key");
    disp.register::<SigningKeyCredential>()
        .expect("disp: signing_key");

    // Collect key sets for the two KEY-keyed registries.
    let cred_keys: HashSet<String> = cred.iter_keys().map(str::to_owned).collect();
    let disp_keys: HashSet<String> = disp.iter_keys().map(str::to_owned).collect();

    // Exact key-set equality: same Credential::KEY strings in both.
    assert_eq!(
        cred_keys, disp_keys,
        "CredentialRegistry vs CredentialDispatch KEY drift \
         (cred={cred_keys:?}, disp={disp_keys:?})"
    );

    // StateProjectionRegistry uses state-KIND strings — different namespace —
    // so only count equality is meaningful.
    let proj_count = proj.len();
    assert_eq!(
        cred_keys.len(),
        proj_count,
        "CredentialRegistry vs StateProjectionRegistry entry count mismatch \
         (cred_keys={cred_keys:?}, proj_count={proj_count})"
    );
}
