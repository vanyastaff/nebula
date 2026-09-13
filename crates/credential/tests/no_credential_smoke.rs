//! Smoke tests for the `NoCredential` opt-out type.
//!
//! Verifies the basic `Credential` contract works for the no-auth case
//! used by `Resource` impls that don't need credential binding.

use nebula_credential::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialState, NoCredential,
    NoCredentialState, StaticResolveResult,
};
use nebula_schema::{AuthoredValue, schema_of};

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
    let schema = schema_of::<<NoCredential as Credential>::Properties>().unwrap();
    let values = schema
        .validate(AuthoredValue::from_data(serde_json::Value::Null).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert!(values.schema().ptr_eq(&schema));
    assert_eq!(values.to_wire_json(), serde_json::Value::Null);
    let ctx = CredentialContext::for_owner("test-owner");
    values.into_typed::<()>().unwrap();
    let outcome = NoCredential::resolve(&(), &ctx)
        .await
        .expect("NoCredential::resolve never fails");
    // Explicit type annotation keeps the static result type visible.
    // if NoCredentialState ever gains a field (becoming a tuple/struct variant),
    // the bare-name pattern would silently turn into a binding and stop asserting
    // the type. The `let state: NoCredentialState = ...` form fails to compile
    // in that case, surfacing the regression.
    let _state: NoCredentialState = match outcome {
        StaticResolveResult::Complete(s) => s,
        other => panic!("expected StaticResolveResult::Complete, got {other:?}"),
    };
}
