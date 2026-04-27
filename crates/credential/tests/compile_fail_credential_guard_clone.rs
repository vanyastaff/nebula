//! SEC-05 (security hardening 2026-04-27 Stage 2) — `CredentialGuard<S>: !Clone`.
//!
//! `CredentialGuard` previously implemented `Clone` when `S: Zeroize + Clone`.
//! Cloning would create a second zeroize point on the same plaintext,
//! violating PRODUCT_CANON §4.2 invariant N10 («plaintext does not cross
//! spawn boundary»). This probe verifies the impl is gone via fully-qualified
//! syntax (FQS) so `Clone` cannot resolve through `Deref` to the inner `S`.
//!
//! See `docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md` §4.

#[test]
fn compile_fail_credential_guard_clone() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/credential_guard_clone.rs");
}
