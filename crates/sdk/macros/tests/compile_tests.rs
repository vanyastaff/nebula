//! Compile-time tests for nebula-macros.
//!
//! These tests use trybuild to verify that the macros generate
//! the expected code and produce helpful error messages.

#[test]
fn test_action_derive() {
    // Positive tests - should compile successfully
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/action_pass.rs");
}

#[test]
fn test_action_derive_fail() {
    // Negative tests - should produce specific errors
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/action_fail.rs");
}

#[test]
fn test_plugin_derive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/plugin_pass.rs");
}

#[test]
fn test_resource_derive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/resource_pass.rs");
}

// TODO(v2): Re-enable when #[derive(Credential)] is rewritten for v2 Credential trait.
// #[test]
// fn test_credential_derive() { ... }
// #[test]
// fn test_credential_flow_derive() { ... }

#[test]
fn test_parameters_derive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/parameters_pass.rs");
}

#[test]
fn test_validator_derive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/validator_pass.rs");
    t.pass("tests/ui/validator_each_pass.rs");
}

#[test]
fn test_validator_derive_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/validator_fail.rs");
    t.compile_fail("tests/ui/validator_contains_non_string_fail.rs");
    t.compile_fail("tests/ui/validator_is_true_non_bool_fail.rs");
    t.compile_fail("tests/ui/validator_min_size_non_collection_fail.rs");
    t.compile_fail("tests/ui/validator_size_range_non_collection_fail.rs");
    t.compile_fail("tests/ui/validator_length_range_non_string_fail.rs");
    t.compile_fail("tests/ui/validator_each_non_collection_fail.rs");
    t.compile_fail("tests/ui/validator_each_non_string_fail.rs");
    t.compile_fail("tests/ui/validator_each_contains_non_string_fail.rs");
    t.compile_fail("tests/ui/validator_each_regex_non_string_fail.rs");
    t.compile_fail("tests/ui/validator_each_invalid_entry_fail.rs");
}
