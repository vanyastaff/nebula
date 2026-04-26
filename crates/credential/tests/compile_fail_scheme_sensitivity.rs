//! Probe 2 — §15.5 AuthScheme sensitivity dichotomy.
//!
//! Verifies the trait shape rejects:
//! (a) `#[auth_scheme(sensitive)]` with plain `String` for token-named field
//! (b) `#[auth_scheme(public)]` with `SecretString` field
//! (c) `impl SensitiveScheme` without `ZeroizeOnDrop` derive

#[test]
fn compile_fail_scheme_sensitivity() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_sensitivity_plain_string.rs");
    t.compile_fail("tests/probes/scheme_sensitivity_public_with_secret.rs");
    t.compile_fail("tests/probes/scheme_sensitivity_no_zeroize.rs");
}
