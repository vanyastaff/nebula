//! Probe 8 fixture — Tech Spec §15.8 capability-from-type.
//!
//! `CredentialMetadata::capabilities_enabled` was removed when §15.8
//! moved capability discovery from the self-attested metadata field to
//! the registration-time fold over `plugin_capability_report::IsX::VALUE`
//! constants. Reading the absent field is E0609 — surfaces the
//! regression at compile time if anything ever reintroduces the knob.

use nebula_credential::{AuthPattern, CredentialMetadata};

fn read_capabilities_field(meta: &CredentialMetadata) {
    // Pattern is intentionally legitimate (`pattern` is a real `pub` field
    // on `CredentialMetadata`) so the only compile error this probe surfaces
    // is the absent `capabilities_enabled` — proving §15.8 removed the field
    // without masking the diagnostic via an unrelated `private field` error.
    //
    // If `CredentialMetadata::pattern` ever loses its `pub` visibility, this
    // probe will fail with TWO errors (private + missing) instead of the
    // one we're targeting. Reviewers should keep `pattern` `pub` or update
    // this fixture to exercise a different `pub` field.
    let _: &AuthPattern = &meta.pattern;
    // The next read fails with E0609 because §15.8 deleted the field.
    let _ = &meta.capabilities_enabled;
}

fn main() {
    let _ = read_capabilities_field;
}
