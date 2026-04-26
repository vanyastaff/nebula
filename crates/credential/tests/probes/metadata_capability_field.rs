//! Probe 8 fixture — Tech Spec §15.8 capability-from-type.
//!
//! `CredentialMetadata::capabilities_enabled` was removed when §15.8
//! moved capability discovery from the self-attested metadata field to
//! the registration-time fold over `plugin_capability_report::IsX::VALUE`
//! constants. Reading the absent field is E0609 — surfaces the
//! regression at compile time if anything ever reintroduces the knob.

use nebula_credential::{AuthPattern, CredentialMetadata};

fn read_capabilities_field(meta: &CredentialMetadata) {
    // Pattern is intentionally legitimate (`pattern` is a real field) so
    // the only compile error is the absent `capabilities_enabled`.
    let _: &AuthPattern = &meta.pattern;
    // The next read fails with E0609 because §15.8 deleted the field.
    let _ = &meta.capabilities_enabled;
}

fn main() {
    let _ = read_capabilities_field;
}
