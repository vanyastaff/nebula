//! UI tests for `#[derive(Validator)]` diagnostics.
//!
//! These use `trybuild` to assert that common misuses produce actionable
//! compile errors. Run `TRYBUILD=overwrite cargo test --test ui` to
//! regenerate the expected `.stderr` files after diagnostic changes.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
