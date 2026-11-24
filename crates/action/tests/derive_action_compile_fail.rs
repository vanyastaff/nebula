//! Compile-fail / compile-pass probes for `#[derive(Action)]` (Variant A).
//!
//! Each probe under `tests/probes/derive_*.rs` exercises one diagnostic
//! contract of the macro:
//!
//! - missing `config = ...` / `output = ...` arguments, and the renamed `input = ...`,
//! - two `#[slot]` fields with the same key,
//! - a `#[slot]` field that holds no slot value (a plain value, a
//!   `ResourceGuard<R>` lease, a `Lazy<...>`, a bare `CredentialGuard<S>`),
//! - the replaced `#[resource]` / `#[credential]` field attributes,
//! - unknown keys inside `#[action(...)]`,
//! - a value assigned to the `read_only` flag,
//! - the flag's pre-0.22.0 spelling, refused with a hint,
//! - tuple structs.
//!
//! The positive probe (`tests/probes/derive_positive_guard_shapes.rs`)
//! compiles slot-free structs and every slot shape an action holds.

#[test]
fn derive_action_compile_fail_probes() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_missing_config.rs");
    t.compile_fail("tests/probes/derive_renamed_input.rs");
    t.compile_fail("tests/probes/derive_missing_output.rs");
    t.compile_fail("tests/probes/derive_unknown_attr_key.rs");
    t.compile_fail("tests/probes/derive_read_only_value.rs");
    t.compile_fail("tests/probes/derive_renamed_effect_flag.rs");
    t.compile_fail("tests/probes/derive_conflicting_slot_keys.rs");
    t.compile_fail("tests/probes/derive_slot_on_wrong_type.rs");
    t.compile_fail("tests/probes/derive_replaced_slot_attributes.rs");
    t.compile_fail("tests/probes/derive_tuple_struct.rs");
}

#[test]
fn derive_action_compile_pass_positive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/probes/derive_positive_guard_shapes.rs");
}
