#[test]
fn value_phases_cannot_supply_schema() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/value_schema_compile_fail/*.rs");
}
