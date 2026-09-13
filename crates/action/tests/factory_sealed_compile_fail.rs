//! Compile-fail proof that executable factories are created only by `nebula-action`.

#[test]
fn action_factory_cannot_be_implemented_downstream() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/probes/action_factory_direct_impl.rs");
}
