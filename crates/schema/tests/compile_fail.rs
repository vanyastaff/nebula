#[test]
fn typed_builder_compile_fail_guards() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
