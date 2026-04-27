//! SEC-11 (security hardening 2026-04-27 Stage 1) — bare `encrypt` removed
//! from public surface.
//!
//! Verifies external callers cannot construct legacy (no-AAD) envelopes via
//! `nebula_credential::secrets::crypto::encrypt` or via the prelude
//! re-export. Production callers must go through `encrypt_with_aad` /
//! `encrypt_with_key_id` — the AAD-mandatory paths.
//!
//! See `docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md` §3.

#[test]
fn compile_fail_encrypt_no_aad_removed() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/encrypt_no_aad_removed.rs");
}
