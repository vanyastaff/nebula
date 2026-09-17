//! `core.json_transform` — apply a sequence of transform operations to a JSON object.
//!
//! Each [`TransformOperation`] in the `operations` list is applied left-to-right to the
//! running object. Operations compose: `Rename { from: "a", to: "b" }` followed by
//! `Rename { from: "b", to: "c" }` produces a key named `"c"` carrying the original
//! value of `"a"`.
//!
//! ## Scope
//!
//! `pick`, `omit`, and `rename` act on **top-level keys only**: for them, dot characters in
//! key names are literal characters, not path separators — `"a.b"` refers to a single key
//! named `"a.b"`, not a nested path.
//!
//! `flatten` reaches deeper: it collapses nested objects into dotted top-level keys
//! (`{"a":{"b":1}}` → `{"a.b":1}`). A `flatten` placed before a `pick`/`omit`/`rename`
//! therefore lets those top-level operations address the now-flattened keys (e.g. pick
//! `"a.b"` after a `flatten`). See [`TransformOperation::Flatten`] for the full contract.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data":       { /* optional base object */ },
//!   "operations": [
//!     { "op": "pick",    "fields": ["a", "b"] },
//!     { "op": "omit",    "fields": ["secret"] },
//!     { "op": "rename",  "from": "old_name", "to": "new_name" },
//!     { "op": "flatten", "separator": "." }
//!   ]
//! }
//! ```
//!
//! ## Output
//!
//! The transformed JSON object.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{
    HasSchema, ListField, Property, Schema, ValidSchema, ValidationReport, field_key,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::instrument;

use crate::util::ValueTypeNameStr;

// ── Config types ──────────────────────────────────────────────────────────────

/// Default key separator for `Flatten` when `separator` is omitted: a single dot.
fn default_separator() -> String {
    ".".to_owned()
}

/// A single transform step applied to the running JSON object.
///
/// Operations are applied in declaration order. Forward-compatibility for new
/// optional fields is handled via `#[serde(default)]` in future versions, not
/// `#[non_exhaustive]`, because these types are deserialized from workflow JSON
/// rather than literal-constructed by external Rust code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TransformOperation {
    /// Retain only the listed keys; all other keys are removed.
    ///
    /// Missing keys are silently skipped. An empty `fields` list produces an
    /// empty object.
    Pick {
        /// Top-level keys to retain; all other keys are dropped.
        fields: Vec<String>,
    },

    /// Remove the listed keys from the object.
    ///
    /// Missing keys are silently ignored. An empty `fields` list leaves the
    /// object unchanged.
    Omit {
        /// Top-level keys to remove.
        fields: Vec<String>,
    },

    /// Move the value at `from` to the key `to`, removing `from`.
    ///
    /// - If `from` is absent, the operation returns a fatal error.
    /// - If `to` already exists, its previous value is overwritten.
    /// - If `from == to`, the operation is a no-op.
    Rename {
        /// Source key whose value is moved.
        from: String,
        /// Destination key that receives the value.
        to: String,
    },

    /// Collapse nested objects into dotted top-level keys.
    ///
    /// Each leaf value's full path is joined by `separator` into a single
    /// top-level key: `{"a":{"b":{"c":1}}}` with `"."` → `{"a.b.c":1}`.
    /// After a `Flatten`, a following `pick`/`omit`/`rename` addresses the
    /// flattened keys (e.g. `pick(["a.b"])`) — that composition is the point.
    ///
    /// ## What counts as a leaf
    ///
    /// - **Scalars** (null, bool, number, string) are leaves: `{"a":1}` → `{"a":1}`.
    /// - **Arrays are leaves** — they are *not* descended into and never produce
    ///   index keys: `{"a":[1,2]}` → `{"a":[1,2]}`, never `{"a.0":1,"a.1":2}`.
    /// - **Empty objects are leaves** — an object with no keys has no descendable
    ///   path, so its key is preserved mapping to `{}`: `{"a":{}}` → `{"a":{}}`.
    ///
    /// ## Key collisions
    ///
    /// When two source paths flatten to the same dotted key (e.g.
    /// `{"a":{"b":1},"a.b":2}` both target `"a.b"`), the operation is total: it
    /// does not error. Last-writer-wins, matching `serde_json` object-insert
    /// semantics — the value written last into the rebuilt map survives. A path
    /// descended from a nested object is written after sibling literal-dotted
    /// keys, so for `{"a":{"b":1},"a.b":2}` the descended `1` wins.
    Flatten {
        /// String joining nested path segments into a single top-level key.
        /// Defaults to `"."`.
        #[serde(default = "default_separator")]
        separator: String,
    },
}

/// Resolved input for `JsonTransform`.
///
/// The engine resolves `NodeDefinition::parameters` into this struct before
/// dispatching. `data` defaults to an empty object when absent or `null`.
/// Operations are applied in declaration order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonTransformInput {
    /// Base object to transform. `null` / absent → empty object.
    #[serde(default)]
    pub data: Option<Value>,
    /// Ordered list of transform operations applied left-to-right.
    #[serde(default)]
    pub operations: Vec<TransformOperation>,
}

impl HasSchema for JsonTransformInput {
    #[instrument(name = "core.json_transform.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA
            .get_or_init(|| {
                Schema::builder()
                    .property(super::input_schema::nullable_object_data())
                    .property(operations_schema())
                    .build()
            })
            .clone()
    }
}

/// Both transform actions consume the same internally tagged operation records.
/// Presence of variant-specific fields stays with serde, preserving valid empty
/// strings and arrays that form-style `required` would otherwise reject.
pub(super) fn operations_schema() -> ListField {
    Property::list(field_key!("operations")).item(
        Property::object(field_key!("item"))
            .description("Tagged transform; variant-specific field presence is checked by serde.")
            .property(
                Property::select(field_key!("op"))
                    .option("pick", "Pick")
                    .option("omit", "Omit")
                    .option("rename", "Rename")
                    .option("flatten", "Flatten")
                    .required(),
            )
            .property(super::input_schema::strings(field_key!("fields")))
            .property(Property::string(field_key!("from")))
            .property(Property::string(field_key!("to")))
            .property(Property::string(field_key!("separator"))),
    )
}

// ── Shared operation applier ──────────────────────────────────────────────────

/// Apply a sequence of [`TransformOperation`]s left-to-right to a single JSON
/// object map in place.
///
/// This is the shared implementation used by both `core.json_transform` (one
/// object) and `core.map` (one object per array element). Keeping the loop
/// here prevents duplication and ensures both actions have identical per-object
/// semantics.
///
/// `context` is the calling action's key (e.g. `"json_transform"` / `"map"`);
/// it prefixes any error so the message names the action that actually faulted.
///
/// `Pick` rebuilds the object with keys in the order they appear in its `fields`
/// list (declaration order), not the source object's original order. `Flatten`
/// rebuilds the object with dotted leaf keys (see [`flatten_object`]).
///
/// # Errors
///
/// Returns [`ActionError::Fatal`] when a `Rename` operation references a source
/// key that is absent from `target`. `Pick` and `Omit` missing keys are silent
/// no-ops per the documented contract. `Flatten` never errors — it is total.
pub(crate) fn apply_operations(
    target: &mut Map<String, Value>,
    operations: &[TransformOperation],
    context: &str,
) -> Result<(), ActionError> {
    for operation in operations {
        match operation {
            TransformOperation::Pick { fields } => {
                let retained: Map<String, Value> = fields
                    .iter()
                    .filter_map(|key| {
                        let value = target.remove(key.as_str())?;
                        Some((key.clone(), value))
                    })
                    .collect();
                *target = retained;
            },
            TransformOperation::Omit { fields } => {
                for key in fields {
                    target.remove(key.as_str());
                }
            },
            TransformOperation::Rename { from, to } => {
                if from == to {
                    // Source and destination are the same key — nothing to move.
                    continue;
                }
                let moved_value = target.remove(from.as_str()).ok_or_else(|| {
                    ActionError::fatal(format!(
                        "{context}: rename source key `{from}` not found in object"
                    ))
                })?;
                target.insert(to.clone(), moved_value);
            },
            TransformOperation::Flatten { separator } => {
                *target = flatten_object(std::mem::take(target), separator);
            },
        }
    }
    Ok(())
}

/// Collapse a JSON object's nested objects into dotted top-level keys.
///
/// Uses an explicit stack worklist rather than recursion so that pathologically
/// deep input cannot overflow the call stack — depth is bounded only by the
/// nesting `serde_json` already parsed, but the iterative form makes that
/// independence explicit and total.
///
/// Leaf classification (a leaf is emitted at its accumulated dotted prefix):
/// scalars, arrays, and *empty* objects are leaves; only non-empty objects are
/// descended into. On a dotted-key collision the later write wins, matching
/// `serde_json` object-insert semantics.
fn flatten_object(source: Map<String, Value>, separator: &str) -> Map<String, Value> {
    let mut flattened = Map::new();
    // Worklist of (accumulated-prefix, value-to-place), walked with an advancing
    // cursor. Seed with the top-level entries; descending a non-empty object
    // appends its children (prefix extended by `separator`) to the back, so deep
    // paths are inserted after shallower siblings — giving the documented
    // last-writer-wins on a dotted-key collision.
    let mut work: Vec<(String, Value)> = source.into_iter().collect();
    let mut index = 0;
    while index < work.len() {
        // Take ownership of this entry's value without shifting the vec; the
        // placeholder is never revisited because the cursor only moves forward.
        let (prefix, value) = std::mem::replace(&mut work[index], (String::new(), Value::Null));
        index += 1;
        match value {
            Value::Object(inner) if !inner.is_empty() => {
                for (child_key, child_value) in inner {
                    let child_prefix = format!("{prefix}{separator}{child_key}");
                    work.push((child_prefix, child_value));
                }
            },
            // Leaf: scalar, array, or empty object — place at the accumulated key.
            leaf => {
                flattened.insert(prefix, leaf);
            },
        }
    }
    flattened
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that applies a sequence of transform operations to a JSON object.
///
/// Keyed `core.json_transform`. No I/O, no credentials, no resources.
///
/// # Example
///
/// Operations serialize to a tagged JSON object; the `"op"` field drives
/// deserialization back to the correct variant:
///
/// ```rust
/// use nebula_plugin_core::actions::json_transform::TransformOperation;
/// use serde_json::json;
///
/// let op = TransformOperation::Pick { fields: vec!["a".into(), "b".into()] };
///
/// // Wire shape: {"op":"pick","fields":["a","b"]}
/// let wire = serde_json::to_value(&op).unwrap();
/// assert_eq!(wire, json!({"op": "pick", "fields": ["a", "b"]}));
///
/// // Round-trip: deserialize back to the same variant
/// let restored: TransformOperation = serde_json::from_value(wire).unwrap();
/// assert_eq!(restored, op);
/// ```
///
/// Wire the action into the engine via [`CorePlugin`](crate::CorePlugin) and
/// `WorkflowEngine::with_plugin` — see the crate-level docs for a complete
/// wiring example.
#[derive(Debug)]
pub struct JsonTransform;

impl nebula_action::action::Action for JsonTransform {
    type Input = JsonTransformInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.json_transform"),
            nebula_action::metadata_name!("JSON Transform"),
            "Applies a sequence of pick/omit/rename/flatten operations to a JSON object",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for JsonTransform {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(JsonTransform)
    }
}

impl StatelessAction for JsonTransform {
    #[instrument(
        name = "core.json_transform",
        skip_all,
        fields(operation_count = input.operations.len())
    )]
    async fn execute(
        &self,
        input: JsonTransformInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        let mut working_fields: Map<String, Value> = match input.data {
            Some(Value::Object(map)) => map,
            Some(Value::Null) | None => Map::new(),
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "json_transform: `data` must be a JSON object or null, got {}",
                    other.type_name_str()
                )));
            },
        };

        apply_operations(&mut working_fields, &input.operations, "json_transform")?;

        Ok(ActionResult::success(Value::Object(working_fields)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "json_transform_tests.rs"]
mod tests;
