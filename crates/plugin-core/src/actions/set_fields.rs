//! `core.set_fields` — merge a list of field assignments onto a JSON object.
//!
//! Semantics mirror the n8n "Edit Fields / Set" node: each `Assignment` in
//! the `assignments` list names a top-level key and supplies a JSON value.
//! The action merges those pairs onto the `data` object (defaulting to an
//! empty object) and returns the merged result.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data":        { /* optional base object */ },
//!   "assignments": [ { "name": "field_name", "value": <any JSON> }, ... ]
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! { "field_name": <value>, ...merged }
//! ```
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::instrument;

// ── Config types ──────────────────────────────────────────────────────────────

/// A single field assignment: `name → value`.
///
/// Both fields are always present on the wire; forward-compatibility for new
/// optional fields is handled via `#[serde(default)]` in future versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    /// Top-level key to set on the output object.
    pub name: String,
    /// JSON value to assign.
    pub value: Value,
}

/// Resolved input for `SetFields`.
///
/// The engine resolves `NodeDefinition::parameters` into this struct before
/// dispatching. The `data` field is optional; if absent or `null` the
/// assignments are merged onto an empty object.
///
/// Forward-compatibility for new optional fields is handled via
/// `#[serde(default)]` on any additions, not `#[non_exhaustive]`, because
/// these types are deserialized from workflow JSON rather than literal-
/// constructed by external Rust code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetFieldsInput {
    /// Base object to merge assignments onto. `null` / absent → empty object.
    #[serde(default)]
    pub data: Option<Value>,
    /// Ordered list of field assignments applied left-to-right.
    #[serde(default)]
    pub assignments: Vec<Assignment>,
}

// `assignments[*].value` is a fully dynamic JSON value, so a concrete typed
// schema cannot enumerate its shape. Empty schema is the honest declaration;
// the doc-comment above describes the expected structure out-of-band.
impl HasSchema for SetFieldsInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that merges a list of field assignments onto a JSON object.
///
/// Keyed `core.set_fields`. No I/O, no credentials, no resources.
///
/// # Example
///
/// Build the input by constructing `SetFieldsInput` directly (or deserialize
/// it from workflow-node JSON via `serde_json`):
///
/// ```rust
/// use nebula_plugin_core::actions::set_fields::{Assignment, SetFieldsInput};
/// use serde_json::json;
///
/// let input = SetFieldsInput {
///     data: Some(json!({"existing": 1})),
///     assignments: vec![
///         Assignment { name: "new_field".into(), value: json!(42) },
///     ],
/// };
/// assert_eq!(input.assignments.len(), 1);
/// ```
///
/// Wire the action into the engine via [`CorePlugin`](crate::CorePlugin) and
/// `WorkflowEngine::with_plugin` — see the crate-level docs for a complete
/// wiring example.
#[derive(Debug)]
pub struct SetFields;

impl nebula_action::action::Action for SetFields {
    type Input = SetFieldsInput;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.set_fields"),
            "Set Fields",
            "Merges a list of named field assignments onto a JSON object",
        )
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for SetFields {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(SetFields)
    }
}

impl StatelessAction for SetFields {
    #[instrument(
        name = "core.set_fields",
        skip_all,
        fields(assignment_count = input.assignments.len())
    )]
    async fn execute(
        &self,
        input: SetFieldsInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        let mut out: Map<String, Value> = match input.data {
            Some(Value::Object(map)) => map,
            Some(Value::Null) | None => Map::new(),
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "set_fields: `data` must be a JSON object or null, got {}",
                    other.type_name_str()
                )));
            },
        };

        for Assignment { name, value } in input.assignments {
            out.insert(name, value);
        }

        Ok(ActionResult::success(Value::Object(out)))
    }
}

// ── Helper trait (local extension) ───────────────────────────────────────────

/// Local extension on `serde_json::Value` to get a readable type name for
/// error messages without pulling in an extra crate.
trait ValueTypeNameStr {
    fn type_name_str(&self) -> &'static str;
}

impl ValueTypeNameStr for Value {
    fn type_name_str(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use nebula_action::testing::TestContextBuilder;
    use serde_json::json;

    use super::*;

    fn ctx() -> impl ActionContext {
        TestContextBuilder::new().build()
    }

    #[tokio::test]
    async fn merges_assignments_onto_empty_base() {
        let action = SetFields;
        let input = SetFieldsInput {
            data: None,
            assignments: vec![
                Assignment {
                    name: "a".into(),
                    value: json!(1),
                },
                Assignment {
                    name: "b".into(),
                    value: json!("hello"),
                },
            ],
        };
        let result = action.execute(input, &ctx()).await.unwrap();
        let out = result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .unwrap();
        assert_eq!(out["a"], json!(1));
        assert_eq!(out["b"], json!("hello"));
    }

    #[tokio::test]
    async fn assignments_overlay_existing_keys() {
        let action = SetFields;
        let input = SetFieldsInput {
            data: Some(json!({"existing": true, "overwritten": "old"})),
            assignments: vec![Assignment {
                name: "overwritten".into(),
                value: json!("new"),
            }],
        };
        let result = action.execute(input, &ctx()).await.unwrap();
        let out = result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .unwrap();
        assert_eq!(out["existing"], json!(true));
        assert_eq!(out["overwritten"], json!("new"));
    }

    #[tokio::test]
    async fn null_data_treated_as_empty_object() {
        let action = SetFields;
        let input = SetFieldsInput {
            data: Some(Value::Null),
            assignments: vec![Assignment {
                name: "x".into(),
                value: json!(99),
            }],
        };
        let result = action.execute(input, &ctx()).await.unwrap();
        let out = result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .unwrap();
        assert_eq!(out["x"], json!(99));
    }

    #[tokio::test]
    async fn non_object_data_returns_fatal_error() {
        let action = SetFields;
        let input = SetFieldsInput {
            data: Some(json!([1, 2, 3])),
            assignments: vec![],
        };
        let err = action.execute(input, &ctx()).await.unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    #[tokio::test]
    async fn empty_assignments_returns_data_unchanged() {
        let action = SetFields;
        let data = json!({"keep": "this"});
        let input = SetFieldsInput {
            data: Some(data.clone()),
            assignments: vec![],
        };
        let result = action.execute(input, &ctx()).await.unwrap();
        let out = result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn action_key_is_core_dot_set_fields() {
        use nebula_action::action::Action;
        assert_eq!(SetFields::metadata().base.key.as_str(), "core.set_fields");
    }
}
