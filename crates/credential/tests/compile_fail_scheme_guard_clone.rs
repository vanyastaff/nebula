//! Probe 7 — `SchemeGuard<'a, C>` is `!Clone`.
//!
//! Per Tech Spec §15.7: `SchemeGuard` must not be cloneable so plaintext
//! scheme material cannot be silently duplicated into side channels.
//! The fixture invokes `Clone::clone` on `SchemeGuard` via fully-qualified
//! syntax (so trait resolution cannot autoderef to the inner `Scheme`)
//! and expects `E0277` ("the trait `Clone` is not implemented for
//! `SchemeGuard<...>`"). Stage 4 fix wave switched the probe from
//! method-syntax `g.clone()` (which would have yielded `E0599`) to FQS
//! to lock the trait-level "guard is not Clone" invariant rather than
//! the surface-level method lookup.

#[test]
fn compile_fail_scheme_guard_clone() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_guard_clone.rs");
}
