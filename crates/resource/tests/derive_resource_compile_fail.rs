//! Compile-fail / compile-pass probes for `#[derive(Resource)]`
//! (slot model).
//!
//! Each compile-fail probe under `tests/probes/derive_*.rs` exercises one
//! diagnostic contract of the macro:
//!
//! - missing `config = ...` argument,
//! - missing `topology = "..."` argument,
//! - invalid `topology = "..."` value (not in {pool, resident}),
//! - unknown keys inside `#[resource(...)]`,
//! - tuple struct rejection.
//!
//! The positive probe (`tests/probes/derive_positive_unit_resource.rs`)
//! exercises a clean derive expansion against a minimal `ResourceConfig`
//! (`Pool` topology, all attribute defaults exercised).

#[test]
fn derive_resource_compile_fail_probes() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_missing_config.rs");
    t.compile_fail("tests/probes/derive_missing_topology.rs");
    t.compile_fail("tests/probes/derive_invalid_topology.rs");
    t.compile_fail("tests/probes/derive_unknown_attr_key.rs");
    t.compile_fail("tests/probes/derive_tuple_struct.rs");
    t.compile_fail("tests/probes/derive_slot_option_wrapped.rs");
}

#[test]
fn derive_resource_compile_pass_positive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/probes/derive_positive_unit_resource.rs");
}

/// `topology = "bounded"` is now rejected — the Bounded topology was
/// removed. The accepted set is `pool` / `resident` only; any other
/// string, including the former `bounded`, produces a compile error.
#[test]
fn derive_resource_rejects_bounded_topology() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/derive_bounded_topology_rejected.rs");
}

#[test]
fn derive_emits_slot_accessor() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/derive_slot_accessor.rs");
}

/// `topology = "bounded"` used to be the third accepted string alongside
/// `pool` / `resident`; verify it now appears in the invalid-topology
/// error message as just the two-element set.
#[test]
fn derive_invalid_topology_error_lists_two_variants() {
    let t = trybuild::TestCases::new();
    // `derive_invalid_topology.rs` uses `topology = "wat"` which hits the
    // same error-message branch; the .stderr golden pins the exact wording.
    t.compile_fail("tests/probes/derive_invalid_topology.rs");
}
