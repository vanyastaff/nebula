//! Compile-fail checks for lifecycle authority and admitted-definition immutability.

#[test]
fn lifecycle_authority_is_not_publicly_callable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/probes/managed_handle_is_private.rs");
    tests.compile_fail("tests/probes/managed_resource_view_cannot_close.rs");
    tests.compile_fail("tests/probes/retained_store_has_no_terminal_authority.rs");
    tests.compile_fail("tests/probes/retained_store_has_no_callback_access.rs");
    tests.compile_fail("tests/probes/provider_has_no_shutdown_hook.rs");
    tests.compile_fail("tests/probes/resource_metadata_fields_are_private.rs");
    tests.compile_fail("tests/probes/resource_metadata_is_not_deserialize.rs");
    tests.compile_fail("tests/probes/resource_config_input_rejects_valid_values.rs");
    tests.compile_fail("tests/probes/resource_config_input_rejects_resolved_values.rs");
    tests.compile_fail("tests/probes/resource_factory_direct_impl.rs");
}
