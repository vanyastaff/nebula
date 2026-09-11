//! Integration tests for `#[derive(Action)]` macro (Variant A).
//!
//! Tests verify that the macro correctly emits the `Action` trait impl
//! plus a `FromWorkflowNode` factory body that resolves slot fields.

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionResult, StatelessAction,
};
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

impl StatelessAction for NoCredAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

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
    let factory = nebula_action::GenericStatelessFactory::<NoCredAction>::new()
        .expect("valid test catalog definition");
    let meta = factory.metadata();
    assert_eq!(meta.base().key().as_str(), "test.no_cred");
    assert_eq!(meta.base().name().to_owned(), "No Cred");
    assert_eq!(meta.base().description().to_owned(), "no credentials");
}

#[test]
fn input_schema_derives_from_input_via_schema_of() {
    // P3: there is no `Action::input_schema()` method. The action's
    // input schema is reached through the `Input: HasSchema` associated-type
    // bound via `nebula_schema::schema_of` — the single source of truth.
    let schema = nebula_schema::schema_of::<<NoCredAction as Action>::Input>()
        .expect("valid test catalog definition");
    let direct = <serde_json::Value as HasSchema>::schema().expect("valid test catalog definition");
    assert_eq!(schema, direct);
}

// -- Default name + description (omitted attrs) ----------------------------

#[derive(Action)]
#[action(
    key = "test.defaults",
    input = serde_json::Value,
    output = serde_json::Value
)]
/// Default action description.
struct DefaultsAction;

impl StatelessAction for DefaultsAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn name_defaults_to_struct_name() {
    let factory = nebula_action::GenericStatelessFactory::<DefaultsAction>::new()
        .expect("valid test catalog definition");
    let meta = factory.metadata();
    assert_eq!(meta.base().name().to_owned(), "DefaultsAction");
    assert_eq!(meta.base().description(), "Default action description.");
}

// -- Default version --------------------------------------------------------

#[derive(Action)]
#[action(
    key = "test.versioned",
    description = "Versioned action",
    version = "2.5.0",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct VersionedAction;

impl StatelessAction for VersionedAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn explicit_version_is_propagated() {
    let factory = nebula_action::GenericStatelessFactory::<VersionedAction>::new()
        .expect("valid test catalog definition");
    let meta = factory.metadata();
    assert_eq!(meta.base().version().major, 2);
    assert_eq!(meta.base().version().minor, 5);
    assert_eq!(meta.base().version().patch, 0);
}
