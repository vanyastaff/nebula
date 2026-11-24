//! Compile-fail / compile-pass probes for `Resource` and `ResourceConfig` derives
//! (slot model, two-derive pattern).
//!
//! Compile-fail probes under `tests/probes/` exercise diagnostic contracts:
//!
//! - enum rejected at the type-ident span
//! - a missing `#[resource(key, config)]`, or an unknown key in it, rejected
//! - the replaced `#[credential]` field attribute refused with a hint
//! - `#[slot]` on a tuple-struct field rejected with clear span
//! - a `#[slot]` field of a type that is no resource slot value rejected
//! - `#[slot(key = "...")]` invalid key literal rejected at the literal span
//! - `Option<CredentialSlot<C>>` rejected (the slot is already empty-able)
//! - nonempty or tuple configs cannot publish an automatic empty-record schema
//! - a fingerprinted config field naming a hash-ordered set (`HashSet`) is
//!   rejected at the type segment; `HashMap` and skipped sets are accepted
//! - the derive-emitted `<Name>PinnedSlots` keeps its guard `Arc`s private
//!
//! The positive probes exercise a clean two-derive expansion:
//!
//! - slot-less unit struct with hand-written `impl Provider`, pinning `()`
//! - `CredentialSlot<C>` field with derive-emitted `<field>_slot()` accessor,
//!   `SLOT_<KEY>` constant and pinned `<field>()` accessor

#[test]
fn resource_pinned_slots_keep_their_guards_private() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_pinned_slots_are_private.rs");
}

#[test]
fn resource_slots_rejects_enum() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_on_enum.rs");
}

#[test]
fn resource_derive_requires_its_container_attribute() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_requires_resource_attribute.rs");
}

#[test]
fn resource_derive_refuses_the_replaced_credential_attribute() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_credential_attribute_replaced.rs");
}

#[test]
fn resource_slots_rejects_slot_on_tuple_field() {
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
    t.compile_fail("tests/probes/slot_bad_key_literal.rs");
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

#[test]
fn resource_config_requires_real_schema_for_nonempty_and_tuple_structs() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/probes/config_missing_schema.rs");
}

#[test]
fn resource_config_rejects_hash_ordered_set_fields() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/probes/config_unordered_set_field.rs");
}
