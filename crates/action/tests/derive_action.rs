//! Integration tests for `#[derive(Action)]` macro (Variant A).
//!
//! Tests verify that the macro correctly emits the `Action` trait impl
//! plus a `FromWorkflowNode` factory body that resolves slot fields.

use nebula_action::Action;
use nebula_schema::HasSchema;

// -- No slot fields ---------------------------------------------------------

#[derive(Action)]
#[action(
    key = "test.no_cred",
    name = "No Cred",
    description = "no credentials",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct NoCredAction;

#[test]
fn no_credentials_returns_empty_slot_fields() {
    assert!(NoCredAction::dependencies().slot_fields().is_empty());
}

#[test]
fn no_resources_in_dependencies() {
    assert!(NoCredAction::dependencies().resources().is_empty());
}

#[test]
fn metadata_key_matches_attribute() {
    let meta = NoCredAction::metadata();
    assert_eq!(meta.base.key.as_str(), "test.no_cred");
    assert_eq!(meta.base.name, "No Cred");
    assert_eq!(meta.base.description, "no credentials");
}

#[test]
fn input_schema_derives_from_input_via_schema_of() {
    // P3: there is no `Action::input_schema()` method. The action's
    // input schema is reached through the `Input: HasSchema` associated-type
    // bound via `nebula_schema::schema_of` — the single source of truth.
    let schema = nebula_schema::schema_of::<<NoCredAction as Action>::Input>();
    let direct = <serde_json::Value as HasSchema>::schema();
    assert_eq!(schema, direct);
}

// -- Default name + description (omitted attrs) ----------------------------

#[derive(Action)]
#[action(
    key = "test.defaults",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct DefaultsAction;

#[test]
fn name_defaults_to_struct_name() {
    let meta = DefaultsAction::metadata();
    assert_eq!(meta.base.name, "DefaultsAction");
    assert_eq!(meta.base.description, "");
}

// -- Default version --------------------------------------------------------

#[derive(Action)]
#[action(
    key = "test.versioned",
    version = "2.5.0",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct VersionedAction;

#[test]
fn explicit_version_is_propagated() {
    let meta = VersionedAction::metadata();
    assert_eq!(meta.base.version.major, 2);
    assert_eq!(meta.base.version.minor, 5);
    assert_eq!(meta.base.version.patch, 0);
}
