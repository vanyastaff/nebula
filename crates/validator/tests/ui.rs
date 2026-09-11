//! UI tests for `#[derive(Validator)]` diagnostics.
//!
//! These use `trybuild` to assert that common misuses produce actionable
//! compile errors. Run `TRYBUILD=overwrite cargo test --test ui` to
//! regenerate the expected `.stderr` files after diagnostic changes.

#[test]
#[cfg(feature = "derive")]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}

#[test]
#[cfg(not(feature = "derive"))]
fn typed_narrowing_without_derive() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/typed_narrowing.rs");
}
