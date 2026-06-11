//! Compile-fail / compile-pass probes for `#[derive(ResourceSlots)]`
//! (slot model, two-derive pattern).
//!
//! Compile-fail probes under `tests/probes/` exercise diagnostic contracts:
//!
//! - enum rejected at the type-ident span
//! - `#[credential]` on a tuple-struct field rejected with clear span
//! - `#[credential]` field with wrong type rejected naming both accepted shapes
//! - `#[credential(key = "...")]` invalid key literal rejected at the literal span
//! - `Option<SlotCell<CredentialGuard<C>>>` rejected (must be the bare shape)
//!
//! The positive probes exercise a clean two-derive expansion:
//!
//! - slot-less unit struct with hand-written `impl Resource`
//! - named slot field with derive-emitted `<field>_slot()` accessor

#[test]
fn resource_slots_rejects_enum() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_on_enum.rs");
}

#[test]
fn resource_slots_rejects_credential_on_tuple_field() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_tuple_struct.rs");
}

#[test]
fn resource_slots_rejects_wrong_field_type() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/slot_field_wrong_type.rs");
}

#[test]
fn resource_slots_rejects_bad_key_literal() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/credential_bad_key_literal.rs");
}

#[test]
fn resource_slots_rejects_option_wrapped_slot() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_slot_option_wrapped.rs");
}

#[test]
fn resource_slots_compile_pass_unit_struct() {
    let t = trybuild::TestCases::new();
    t.pass("tests/probes/derive_positive_unit_resource.rs");
}

#[test]
fn resource_slots_compile_pass_slot_accessor() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/derive_slot_accessor.rs");
}
