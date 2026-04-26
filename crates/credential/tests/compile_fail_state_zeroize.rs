//! Probe 1 — §15.4 amendment: `CredentialState` requires `ZeroizeOnDrop`.
//!
//! Verifies the trait shape rejects an `impl CredentialState for X` where
//! `X` does not derive (or otherwise implement) `ZeroizeOnDrop`. Plaintext
//! state at runtime must drop deterministically per §12.5; the supertrait
//! bound is the compile-time gate.

#[test]
fn compile_fail_state_zeroize() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/state_zeroize_missing.rs");
}
