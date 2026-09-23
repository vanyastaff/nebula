//! A composed service must not expose a second management mutation API.
//! External callers submit CredentialCommand through CredentialController.

#[test]
fn service_mutations_require_the_controller_boundary() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/probes/service_mutations_private.rs");
}
