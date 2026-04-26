//! Probe 7 — `SchemeGuard<'a, C>` is `!Clone`.
//!
//! Per Tech Spec §15.7: `SchemeGuard` must not be cloneable so plaintext
//! scheme material cannot be silently duplicated into side channels.
//! The fixture attempts `g.clone()` on a guard and expects `E0599`
//! ("no method named `clone`").

#[test]
fn compile_fail_scheme_guard_clone() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_guard_clone.rs");
}
