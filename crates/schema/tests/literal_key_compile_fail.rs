#[test]
fn macro_support_cannot_forge_field_keys() {
    trybuild::TestCases::new().compile_fail("tests/literal_key_compile_fail/*.rs");
}
