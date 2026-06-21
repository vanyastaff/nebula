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

use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
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

// `data` is a fully dynamic JSON object and `operations` contain only string
// fields — no closed-form schema can be emitted. Empty schema is the honest
// declaration; the module doc describes the expected structure out-of-band.
impl HasSchema for JsonTransformInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
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

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.json_transform"),
            "JSON Transform",
            "Applies a sequence of pick/omit/rename/flatten operations to a JSON object",
        )
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
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use serde_json::json;

    use super::*;

    fn run(
        input: JsonTransformInput,
    ) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
        let action = JsonTransform;
        let ctx = TestContextBuilder::new().build();
        async move { action.execute(input, &ctx).await }
    }

    fn pick(fields: &[&str]) -> TransformOperation {
        TransformOperation::Pick {
            fields: fields.iter().map(ToString::to_string).collect(),
        }
    }

    fn omit(fields: &[&str]) -> TransformOperation {
        TransformOperation::Omit {
            fields: fields.iter().map(ToString::to_string).collect(),
        }
    }

    fn rename(from: &str, to: &str) -> TransformOperation {
        TransformOperation::Rename {
            from: from.to_string(),
            to: to.to_string(),
        }
    }

    fn flatten(separator: &str) -> TransformOperation {
        TransformOperation::Flatten {
            separator: separator.to_string(),
        }
    }

    fn extract_output(result: ActionResult<Value>) -> Value {
        result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .expect("ActionResult must carry a primary output value")
    }

    // ── 1: Pick keeps exactly the named keys ──────────────────────────────────

    #[tokio::test]
    async fn pick_keeps_named_keys() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 1, "b": 2, "c": 3})),
            operations: vec![pick(&["a", "b"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out["a"], json!(1));
        assert_eq!(out["b"], json!(2));
        assert_eq!(out.get("c"), None, "c must be absent after Pick");
    }

    // ── 2: Pick with a missing key silently skips it ──────────────────────────

    #[tokio::test]
    async fn pick_missing_key_skips_silently() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 1})),
            operations: vec![pick(&["a", "missing"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out["a"], json!(1));
        assert_eq!(out.get("missing"), None);
    }

    // ── 3: Pick with empty fields returns an empty object ─────────────────────

    #[tokio::test]
    async fn pick_empty_fields_returns_empty_object() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 1, "b": 2})),
            operations: vec![pick(&[])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({}));
    }

    // ── 4: Omit removes the named key ─────────────────────────────────────────

    #[tokio::test]
    async fn omit_removes_named_key() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 1, "b": 2})),
            operations: vec![omit(&["b"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out["a"], json!(1));
        assert_eq!(out.get("b"), None, "b must be absent after Omit");
    }

    // ── 5: Omit with a missing key is a no-op ─────────────────────────────────

    #[tokio::test]
    async fn omit_missing_key_is_noop() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 1})),
            operations: vec![omit(&["nonexistent"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out["a"], json!(1));
    }

    // ── 6: Rename moves the value to the new key ──────────────────────────────

    #[tokio::test]
    async fn rename_moves_value_to_new_key() {
        let input = JsonTransformInput {
            data: Some(json!({"old": "value"})),
            operations: vec![rename("old", "new")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out["new"], json!("value"));
        assert_eq!(out.get("old"), None, "old key must be removed after Rename");
    }

    // ── 7: Rename onto an existing key overwrites it ──────────────────────────

    #[tokio::test]
    async fn rename_to_existing_key_overwrites() {
        let input = JsonTransformInput {
            data: Some(json!({"src": "new_value", "dst": "old_value"})),
            operations: vec![rename("src", "dst")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out["dst"], json!("new_value"), "dst must hold src's value");
        assert_eq!(out.get("src"), None, "src must be removed");
    }

    // ── 8: Rename with from == to is a no-op ──────────────────────────────────

    #[tokio::test]
    async fn rename_same_key_is_noop() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 42})),
            operations: vec![rename("a", "a")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out["a"], json!(42));
    }

    // ── 9: Rename with missing source returns a Fatal error ───────────────────
    //
    // RED witness: if the `ok_or_else` / fatal branch is removed, this test
    // panics on the unwrap of a `None` instead of returning `Fatal`.

    #[tokio::test]
    async fn rename_missing_source_returns_fatal() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 1})),
            operations: vec![rename("does_not_exist", "b")],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected ActionError::Fatal for missing rename source; got: {err:?}"
        );
    }

    // ── 10: Operations are applied in declaration order ────────────────────────
    //
    // RED witness: if the operations were applied in reverse order (c removed
    // after rename), the rename would fail because "a" would be absent, or the
    // final key "c" would have the wrong value.

    #[tokio::test]
    async fn operations_applied_in_order() {
        // Omit "c" first, then rename "a" → "c".
        // If order were reversed (rename first), "a"'s value would land at "c",
        // then Omit would remove it — producing an empty object.
        // In the correct order: Omit removes original "c", Rename brings "a" there.
        let input = JsonTransformInput {
            data: Some(json!({"a": "a_value", "b": "b_value", "c": "original_c"})),
            operations: vec![omit(&["c"]), rename("a", "c")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out["c"],
            json!("a_value"),
            "c must hold a's original value after Omit→Rename chain"
        );
        assert_eq!(out["b"], json!("b_value"));
        assert_eq!(out.get("a"), None, "a must be absent after rename");
    }

    // ── 11: Empty operations returns the object unchanged ─────────────────────

    #[tokio::test]
    async fn empty_operations_returns_object_unchanged() {
        let data = json!({"keep": "this", "and": "this_too"});
        let input = JsonTransformInput {
            data: Some(data.clone()),
            operations: vec![],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, data);
    }

    // ── 12: null data is treated as an empty object ───────────────────────────

    #[tokio::test]
    async fn null_data_treated_as_empty_object() {
        let input = JsonTransformInput {
            data: Some(Value::Null),
            operations: vec![omit(&["anything"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({}));
    }

    // ── 13: Non-object data returns a Fatal error ─────────────────────────────

    #[tokio::test]
    async fn non_object_data_returns_fatal() {
        let input = JsonTransformInput {
            data: Some(json!([1, 2, 3])),
            operations: vec![],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected ActionError::Fatal for array data; got: {err:?}"
        );
    }

    // ── Flatten: 1/2/3 levels deep collapse to dotted keys ────────────────────

    #[tokio::test]
    async fn flatten_one_level() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {"b": 1}})),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({"a.b": 1}));
    }

    #[tokio::test]
    async fn flatten_two_levels() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {"b": {"c": 2}}})),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({"a.b.c": 2}));
    }

    #[tokio::test]
    async fn flatten_three_levels_and_mixed_siblings() {
        let input = JsonTransformInput {
            data: Some(json!({
                "a": {"b": {"c": {"d": 3}}},
                "x": {"y": 9},
                "top": 7
            })),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({"a.b.c.d": 3, "x.y": 9, "top": 7}));
    }

    // ── Flatten: custom separator joins the segments ──────────────────────────

    #[tokio::test]
    async fn flatten_custom_separator() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {"b": 1}})),
            operations: vec![flatten("_")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({"a_b": 1}));
    }

    // ── Flatten: arrays are leaves, never descended into ──────────────────────
    //
    // RED witness: an index-based flatten would emit {"a.0":1,"a.1":2}; the
    // array-as-leaf contract requires the array value to pass through verbatim.

    #[tokio::test]
    async fn flatten_array_is_a_leaf() {
        let input = JsonTransformInput {
            data: Some(json!({"a": [1, 2]})),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({"a": [1, 2]}));
        assert_eq!(out.get("a.0"), None, "array must not be flattened by index");
    }

    // ── Flatten: a nested object containing an array stops at the array ───────

    #[tokio::test]
    async fn flatten_descends_objects_but_not_arrays_within() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {"b": [1, 2], "c": 3}})),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({"a.b": [1, 2], "a.c": 3}));
    }

    // ── Flatten: top-level scalars are unchanged ──────────────────────────────

    #[tokio::test]
    async fn flatten_top_level_scalars_unchanged() {
        let input = JsonTransformInput {
            data: Some(json!({"a": 1, "b": "two", "c": true, "d": null})),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({"a": 1, "b": "two", "c": true, "d": null}));
    }

    // ── Flatten: an empty nested object is preserved as a leaf ────────────────
    //
    // Documented choice: an empty object has no descendable path, so its key is
    // preserved mapping to `{}` rather than silently dropped.

    #[tokio::test]
    async fn flatten_empty_nested_object_is_preserved() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {}, "b": {"c": {}}})),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!({"a": {}, "b.c": {}}),
            "empty nested objects are leaves preserved at their accumulated key"
        );
    }

    // ── Flatten: key collision is total and last-writer-wins ──────────────────
    //
    // Both the nested `a.b` (from {"a":{"b":1}}) and the literal-dotted "a.b":2
    // target the same key. The op must NOT error; the descended `1` is written
    // last and survives. RED witness: an erroring impl would make this panic on
    // the unwrap of an Err.

    #[tokio::test]
    async fn flatten_key_collision_last_writer_wins() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {"b": 1}, "a.b": 2})),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!({"a.b": 1}),
            "collision is total; the descended nested value wins (written last)"
        );
    }

    // ── Flatten composition: a following pick selects the flattened key ───────
    //
    // RED witness: if flatten produced index/literal keys instead of "a.b", the
    // pick of "a.b" would find nothing and the output would be empty.

    #[tokio::test]
    async fn flatten_then_pick_selects_flattened_key() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {"b": 1, "c": 2}, "keep": {"me": 3}})),
            operations: vec![flatten("."), pick(&["a.b"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!({"a.b": 1}),
            "pick after flatten must address the flattened dotted key"
        );
    }

    // ── Flatten composition: a following omit drops the flattened key ─────────

    #[tokio::test]
    async fn flatten_then_omit_drops_flattened_key() {
        let input = JsonTransformInput {
            data: Some(json!({"a": {"b": 1, "c": 2}})),
            operations: vec![flatten("."), omit(&["a.b"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!({"a.c": 2}),
            "omit after flatten must drop the flattened dotted key"
        );
    }

    // ── Flatten: deeply-nested input flattens without panicking (depth-safe) ──
    //
    // Builds a 10-level-deep chain l0.l1.…l9 and asserts the single dotted leaf.
    // The iterative worklist guarantees no call-stack growth with depth.

    #[tokio::test]
    async fn flatten_deeply_nested_is_depth_safe() {
        const DEPTH: usize = 10;
        // Build {"l0":{"l1":{…{"l9":"leaf"}…}}} from the inside out.
        let mut value = json!("leaf");
        for level in (0..DEPTH).rev() {
            value = json!({ format!("l{level}"): value });
        }
        let expected_key = (0..DEPTH)
            .map(|level| format!("l{level}"))
            .collect::<Vec<_>>()
            .join(".");

        let input = JsonTransformInput {
            data: Some(value),
            operations: vec![flatten(".")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!({ expected_key: "leaf" }));
    }

    // ── 14: Action key is "core.json_transform" ───────────────────────────────

    #[test]
    fn action_key_is_core_dot_json_transform() {
        use nebula_action::action::Action;
        assert_eq!(
            JsonTransform::metadata().base.key.as_str(),
            "core.json_transform"
        );
    }

    // ── 15: Serde round-trip for each TransformOperation variant ─────────────

    #[test]
    fn serde_roundtrip_pick() {
        let op = TransformOperation::Pick {
            fields: vec!["x".into(), "y".into()],
        };
        let json = serde_json::to_string(&op).unwrap();
        let deserialized: TransformOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, op);
    }

    #[test]
    fn serde_roundtrip_omit() {
        let op = TransformOperation::Omit {
            fields: vec!["secret".into()],
        };
        let json = serde_json::to_string(&op).unwrap();
        let deserialized: TransformOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, op);
    }

    #[test]
    fn serde_roundtrip_rename() {
        let op = TransformOperation::Rename {
            from: "old".into(),
            to: "new".into(),
        };
        let json = serde_json::to_string(&op).unwrap();
        let deserialized: TransformOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, op);
    }

    #[test]
    fn serde_roundtrip_flatten() {
        let op = TransformOperation::Flatten {
            separator: "_".into(),
        };
        let json = serde_json::to_string(&op).unwrap();
        let deserialized: TransformOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, op);
    }

    // The wire shape carries the explicit separator.
    #[test]
    fn serde_flatten_wire_shape_with_separator() {
        let op = TransformOperation::Flatten {
            separator: "_".into(),
        };
        let wire = serde_json::to_value(&op).unwrap();
        assert_eq!(wire, json!({"op": "flatten", "separator": "_"}));
    }

    // `separator` is optional on the wire and defaults to ".".
    #[test]
    fn serde_flatten_default_separator_when_omitted() {
        let op: TransformOperation = serde_json::from_value(json!({"op": "flatten"})).unwrap();
        assert_eq!(
            op,
            TransformOperation::Flatten {
                separator: ".".into()
            },
            "omitted separator must default to \".\""
        );
    }
}
