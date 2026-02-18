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

#[test]
fn test_credential_derive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/credential_pass.rs");
}

#[test]
fn test_parameters_derive() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/parameters_pass.rs");
}
