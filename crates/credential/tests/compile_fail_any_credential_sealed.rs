//! Compile-fail contract: erased credential facts come only from typed credentials.

#[test]
fn downstream_cannot_implement_any_credential_directly() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/probes/any_credential_direct_impl.rs");
}
