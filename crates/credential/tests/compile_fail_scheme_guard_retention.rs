//! Probe 6 — `SchemeGuard<'a, C>` cannot be retained past the engine call.
//!
//! Per Tech Spec §15.7 spike iter-3 secondary finding: the engine passes
//! `SchemeGuard<'a, C>` alongside `&'a CredentialContext` with shared
//! lifetime `'a`. A resource that tries to retain the guard (e.g. by
//! storing it in a struct field whose lifetime exceeds the call's) forces
//! the shared borrow to also outlive the struct, which the borrow checker
//! rejects with `E0597` (or an equivalent lifetime-class error).

#[test]
fn compile_fail_scheme_guard_retention() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_guard_retention.rs");
}
