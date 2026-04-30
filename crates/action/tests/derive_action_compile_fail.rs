//! Compile-fail / compile-pass probes for `#[derive(Action)]` (Variant A).
//!
//! Each probe under `tests/probes/derive_*.rs` exercises one diagnostic
//! contract of the macro:
//!
//! - missing `input = ...` / `output = ...` arguments,
//! - duplicate slot keys across `#[resource]` / `#[credential]` fields,
//! - `#[resource]` on a non-`ResourceGuard` field type,
//! - `#[credential]` on a non-`CredentialGuard` field type,
//! - both `#[resource]` and `#[credential]` on the same field,
//! - unknown keys inside `#[action(...)]`.
//!
//! The positive probe (`tests/probes/derive_positive_guard_shapes.rs`)
//! checks all four allowed guard shapes compile in a single struct:
//!   - `ResourceGuard<R>`,
//!   - `Option<ResourceGuard<R>>`,
//!   - `Lazy<ResourceGuard<R>>`,
//!   - `Option<Lazy<ResourceGuard<R>>>`.

#[test]
fn derive_action_compile_fail_probes() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_missing_input.rs");
    t.compile_fail("tests/probes/derive_missing_output.rs");
    t.compile_fail("tests/probes/derive_unknown_attr_key.rs");
    t.compile_fail("tests/probes/derive_conflicting_slot_keys.rs");
    t.compile_fail("tests/probes/derive_resource_on_wrong_type.rs");
    t.compile_fail("tests/probes/derive_credential_on_wrong_type.rs");
    t.compile_fail("tests/probes/derive_both_resource_and_credential.rs");
    t.compile_fail("tests/probes/derive_tuple_struct.rs");
}

#[test]
fn derive_action_compile_pass_positive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/probes/derive_positive_guard_shapes.rs");
}
