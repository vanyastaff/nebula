//! Seam (ADR-0052 P3) — `Action::input_schema()`/`output_schema()` are
//! removed. Schema is reachable only via the `Input`/`Output: HasSchema`
//! associated-type bound, exposed through `nebula_schema::schema_of`. This
//! locks the convergence: the redundant per-trait schema methods do not
//! resolve. The runtime side (the converged path equals
//! `schema_of::<A::Input>()`) is asserted by
//! `derive_action::input_schema_derives_from_input_via_schema_of`.

#[test]
fn compile_fail_action_input_schema_removed() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/action_input_schema_removed.rs");
}
