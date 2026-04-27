//! Smoke tests for the `NoCredential` opt-out type.
//!
//! Verifies the basic `Credential` contract works for the no-auth case
//! used by `Resource` impls that don't need credential binding.

use nebula_credential::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialState, NoCredential,
    NoCredentialState, ResolveResult,
};
use nebula_schema::FieldValues;

#[test]
fn key_matches_spec() {
    assert_eq!(NoCredential::KEY, "no_credential");
}

#[test]
fn scheme_is_unit_with_noauth_pattern() {
    assert_eq!(
        <<NoCredential as Credential>::Scheme>::pattern(),
        AuthPattern::NoAuth
    );
}

#[test]
fn state_kind_matches_spec() {
    assert_eq!(NoCredentialState::KIND, "no_credential");
    assert_eq!(NoCredentialState::VERSION, 1);
}

#[test]
fn project_returns_unit_scheme() {
    // Compiles iff `Scheme = ()` and `project()` returns it; the call would
    // otherwise be a type error or fail to satisfy the assertion fn signature.
    fn assert_unit_scheme<C: Credential<Scheme = ()>>(_state: &C::State) {
        // No-op — the bound `Scheme = ()` is the assertion.
    }
    assert_unit_scheme::<NoCredential>(&NoCredentialState);
    NoCredential::project(&NoCredentialState);
}

#[tokio::test]
async fn resolve_returns_complete_state() {
    let values = FieldValues::default();
    let ctx = CredentialContext::for_test("test-owner");
    let outcome = NoCredential::resolve(&values, &ctx)
        .await
        .expect("NoCredential::resolve never fails");
    // Explicit type annotation, not `matches!(_, ResolveResult::Complete(NoCredentialState))`:
    // if NoCredentialState ever gains a field (becoming a tuple/struct variant),
    // the bare-name pattern would silently turn into a binding and stop asserting
    // the type. The `let state: NoCredentialState = ...` form fails to compile
    // in that case, surfacing the regression.
    let _state: NoCredentialState = match outcome {
        ResolveResult::Complete(s) => s,
        other => panic!("expected ResolveResult::Complete, got {other:?}"),
    };
}
