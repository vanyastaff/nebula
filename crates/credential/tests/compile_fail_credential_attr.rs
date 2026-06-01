//! Compile-fail probes for the `#[nebula_credential::credential]` attribute
//! macro (ADR-0088 D1).
//!
//! These pin the macro's safety diagnostics: a typo'd capability method is
//! rejected (it cannot silently drop a capability), and the interactive
//! `continue_resolve` / `type Pending` pair cannot be split.

#[test]
fn compile_fail_credential_attr_unrecognized_method() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/cred_attr_unrecognized_method.rs");
}

#[test]
fn compile_fail_credential_attr_continue_resolve_without_pending() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/cred_attr_continue_resolve_without_pending.rs");
}
