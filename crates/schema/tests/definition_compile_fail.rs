#[test]
fn admitted_graph_and_addresses_cannot_be_forged_or_deserialized() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/definition_compile_fail/*.rs");
}
