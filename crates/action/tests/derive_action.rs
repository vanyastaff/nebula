//! Integration tests for `#[derive(Action)]` macro.
//!
//! Tests verify that the macro correctly generates `ActionDependencies`
//! and `Action` trait implementations.

use nebula_action::{Action, ActionDependencies};

// -- No credentials ---------------------------------------------------------

#[derive(Action)]
#[action(key = "test.no_cred", name = "No Cred", description = "no credentials")]
struct NoCredAction;

#[test]
fn no_credentials_returns_empty_type_ids() {
    assert!(NoCredAction::credential_types().is_empty());
}

#[test]
fn no_credentials_returns_none() {
    assert!(NoCredAction::credential().is_none());
}

#[test]
fn no_credentials_returns_empty_resources() {
    assert!(NoCredAction::resources().is_empty());
}

#[test]
fn metadata_key_matches_attribute() {
    let action = NoCredAction;
    let meta = action.metadata();
    assert_eq!(meta.key.as_str(), "test.no_cred");
    assert_eq!(meta.name, "No Cred");
    assert_eq!(meta.description, "no credentials");
}

// -- Struct with fields -----------------------------------------------------

#[derive(Action, Clone, serde::Deserialize)]
#[action(
    key = "with_fields",
    name = "Fields Action",
    description = "action with fields"
)]
struct ActionWithFields {
    url: String,
    timeout: u32,
}

#[test]
fn derive_works_on_struct_with_fields() {
    let action = ActionWithFields {
        url: "https://example.com".into(),
        timeout: 30,
    };
    let meta = action.metadata();
    assert_eq!(meta.key.as_str(), "with_fields");
    assert_eq!(meta.name, "Fields Action");
}

// -- Credential type IDs (compile-time verification) ------------------------
//
// Full integration tests with `#[action(credential = Type)]` require types
// that implement `nebula_credential::Credential` + `Default`. Those are
// tested via the `ScopedCredentialAccessor` and `dependency` module unit
// tests. Here we verify the no-credential default behavior, which exercises
// the derive macro's code path for `credential_types()`.
