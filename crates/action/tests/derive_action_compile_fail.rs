//! Compile-fail / compile-pass probes for `#[derive(Action)]` (Variant A).
//!
//! Each probe under `tests/probes/derive_*.rs` exercises one diagnostic
//! contract of the macro:
//!
//! - missing `input = ...` / `output = ...` arguments,
//! - duplicate slot keys across `#[resource]` / `#[credential]` fields,
//! - `#[resource]` on a non-`ResourceHandle` field type,
//! - `#[credential]` on a non-`CredentialGuard` field type,
//! - both `#[resource]` and `#[credential]` on the same field,
//! - unknown keys inside `#[action(...)]`,
//! - a value assigned to the `read_only` flag,
//! - the flag's pre-0.22.0 spelling, refused with a hint,
//! - `Lazy<ResourceHandle<R>>` (a resource handle acquires nothing to defer),
//! - a field spelled with the handle's pre-0.22.0 name, refused with a hint,
//! - a `ResourceGuard<R>` lease slot in any wrapper, removed in 0.27.0 and
//!   refused with a migration hint.
//!
//! The positive probe (`tests/probes/derive_positive_guard_shapes.rs`) is a
//! smoke pass for slot-free structs; the slot-shape matrix lives in the
//! macro's unit tests and in `tests/derive_action.rs`.

#[test]
fn derive_action_compile_fail_probes() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_missing_input.rs");
    t.compile_fail("tests/probes/derive_missing_output.rs");
    t.compile_fail("tests/probes/derive_unknown_attr_key.rs");
    t.compile_fail("tests/probes/derive_read_only_value.rs");
    t.compile_fail("tests/probes/derive_renamed_effect_flag.rs");
    t.compile_fail("tests/probes/derive_conflicting_slot_keys.rs");
    t.compile_fail("tests/probes/derive_resource_on_wrong_type.rs");
    t.compile_fail("tests/probes/derive_credential_on_wrong_type.rs");
    t.compile_fail("tests/probes/derive_both_resource_and_credential.rs");
    t.compile_fail("tests/probes/derive_tuple_struct.rs");
    t.compile_fail("tests/probes/derive_lazy_resource_handle.rs");
    t.compile_fail("tests/probes/derive_renamed_resource_handle.rs");
    t.compile_fail("tests/probes/derive_removed_resource_guard.rs");
}

#[test]
fn derive_action_compile_pass_positive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/probes/derive_positive_guard_shapes.rs");
}
