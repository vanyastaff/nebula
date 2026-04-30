//! Compile-fail / compile-pass probes for `#[derive(Resource)]`
//! (Phase 4 / ADR-0044 / M6 closure).
//!
//! Each compile-fail probe under `tests/probes/derive_*.rs` exercises one
//! diagnostic contract of the macro:
//!
//! - missing `config = ...` argument,
//! - missing `topology = "..."` argument,
//! - invalid `topology = "..."` value (not in {pool, resident, service, transport, exclusive}),
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
}

#[test]
fn derive_resource_compile_pass_positive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/probes/derive_positive_unit_resource.rs");
}
