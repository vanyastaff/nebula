//! Probe 8 — Tech Spec §15.8 capability-from-type authority shift.
//!
//! Verifies that [`CredentialMetadata`](nebula_credential::CredentialMetadata)
//! does NOT carry a `capabilities_enabled` field. Pre-§15.8 the metadata
//! struct exposed a self-attested `capabilities_enabled: Capabilities`
//! that plugin authors populated via the builder — and which `iter_compatible`
//! filtered on. §15.8 (closes security-lead N6 — silent capability
//! self-attestation) moves capability discovery to a registration-time
//! computation from per-credential `plugin_capability_report::IsX::VALUE`
//! constants, leaving no metadata-side knob a plugin can lie through.
//!
//! The probe attempts to read `metadata.capabilities_enabled` on an
//! existing `CredentialMetadata` instance — `E0609` (no field) is the
//! expected failure. If a future refactor reintroduces the field this
//! probe flips to PASS, surfacing the regression at compile time before
//! it can ship.

#[test]
fn compile_fail_metadata_capability_field() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/metadata_capability_field.rs");
}
