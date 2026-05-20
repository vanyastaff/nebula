//! Compile-fail / compile-pass probes for `#[derive(Resource)]`
//! (slot model).
//!
//! Each compile-fail probe under `tests/probes/derive_*.rs` exercises one
//! diagnostic contract of the macro:
//!
//! - missing `config = ...` argument,
//! - missing `topology = "..."` argument,
//! - invalid `topology = "..."` value (not in {pool, resident,
//!   bounded}),
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

/// `topology = "bounded"` is accepted (the folded topology that replaced
/// the legacy `service` / `transport` / `exclusive` strings), alongside
/// `pool` / `resident`. Each maps to its `TopologyTag` via the emitted
/// informational const — the collapsed 3-tag set.
#[test]
fn derive_resource_accepts_collapsed_topologies() {
    let t = trybuild::TestCases::new();
    t.pass("tests/probes/derive_bounded_topology.rs");
}

#[test]
fn derive_emits_slot_accessor() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/derive_slot_accessor.rs");
}

/// The `Bounded` cap typestate makes "release-bearing cap with no release
/// hook" a compile error: a `Capped<N>` / `Exclusive` resource that omits
/// `impl BoundedRelease` fails the `R: BoundedRelease` bound the runtime
/// requires (the blanket no-op covers only `Cap = Unbounded`). This is the
/// type-enforcement that replaced the old silent `TOKEN_MODE ==` no-op.
#[test]
fn bounded_release_shape_is_type_enforced() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/bounded_capped_without_release.rs");
    t.compile_fail("tests/probes/bounded_exclusive_without_reset.rs");
}
