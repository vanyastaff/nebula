//! `core.map` — reshape every element of a JSON array of objects.
//!
//! Applies a sequence of [`TransformOperation`]s to each element in order,
//! returning a new array of the same length with each element reshaped.
//! This fills the gap left by the `{{ }}` expression language, whose
//! `array.map` builtin requires lambda support that is not yet implemented.
//!
//! ## Scope
//!
//! All operations act on **top-level keys only** of each element. The operation
//! vocabulary is identical to `core.json_transform` — `pick`, `omit`, and
//! `rename`. See [`TransformOperation`] for per-operation semantics.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": [
//!     { "first_name": "Alice", "last": "Smith",  "secret": "x" },
//!     { "first_name": "Bob",   "last": "Jones",  "secret": "y" }
//!   ],
//!   "operations": [
//!     { "op": "omit",   "fields": ["secret"] },
//!     { "op": "rename", "from": "first_name", "to": "name" }
//!   ]
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! [
//!   { "last": "Smith", "name": "Alice" },
//!   { "last": "Jones", "name": "Bob"   }
//! ]
//! ```
//!
//! ## Error semantics
//!
//! - `data` absent / null / non-array → **Fatal** naming the actual type.
//! - `operations` empty → **Fatal** (a no-op map is always an authoring mistake;
//!   consistent with the other sibling array nodes that require their config).
//! - Any array element that is not a JSON object → **Fatal** (explicit
//!   `is_object()` guard; `Value::get` on a non-object returns `None` silently,
//!   which would produce wrong results without the guard).
//! - `Rename` source key absent on an element → **Fatal** (propagated from the
//!   shared `apply_operations` contract; identical to `core.json_transform`).
//! - `Pick` / `Omit` missing keys → silent skip (per `apply_operations` contract).
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::actions::json_transform::{TransformOperation, apply_operations};
use crate::util::ValueTypeNameStr;

// ── Input ─────────────────────────────────────────────────────────────────────

/// Input for `core.map`.
///
/// `data` must be a JSON array of objects. `null` / absent values are rejected
/// with a Fatal error — mapping over a non-array is always an authoring mistake.
///
/// ## Wire shape
///
/// ```json
/// {
///   "data": [
///     { "a": 1, "b": 2 },
///     { "a": 3, "b": 4 }
///   ],
///   "operations": [
///     { "op": "pick", "fields": ["a"] }
///   ]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapInput {
    /// Array of JSON objects to reshape. Must be a JSON array when present.
    #[serde(default)]
    pub data: Option<Value>,
    /// Ordered list of transform operations applied left-to-right to each element.
    pub operations: Vec<TransformOperation>,
}

// `data` is a fully dynamic array and `operations` contain only string fields —
// no closed-form schema can be emitted. Empty schema is the honest declaration;
// the module doc describes the expected structure out-of-band.
impl HasSchema for MapInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that reshapes every element of a JSON array of objects by
/// applying a sequence of `pick`/`omit`/`rename` operations to each one.
///
/// Keyed `core.map`. Count-preserving: N elements in → N elements out, original
/// order maintained. No I/O, no credentials, no resources.
///
/// ## Example wire input / output
///
/// ```json
/// {
///   "data": [
///     { "id": 1, "name": "Alice", "secret": "x" },
///     { "id": 2, "name": "Bob",   "secret": "y" }
///   ],
///   "operations": [
///     { "op": "omit",   "fields": ["secret"] },
///     { "op": "rename", "from": "name", "to": "label" }
///   ]
/// }
/// ```
///
/// Output:
///
/// ```json
/// [
///   { "id": 1, "label": "Alice" },
///   { "id": 2, "label": "Bob"   }
/// ]
/// ```
#[derive(Debug)]
pub struct MapAction;

impl nebula_action::action::Action for MapAction {
    type Input = MapInput;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.map"),
            "Map",
            "Reshape each element of a JSON array of objects (per-element pick/omit/rename)",
        )
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for MapAction {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(MapAction)
    }
}

impl StatelessAction for MapAction {
    #[instrument(name = "core.map", skip_all, fields(element_count))]
    async fn execute(
        &self,
        input: MapInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // ── 1. Validate data ──────────────────────────────────────────────────
        let elements: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "map: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "map: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        tracing::Span::current().record("element_count", elements.len());

        // ── 2. Validate operations non-empty ──────────────────────────────────
        //
        // A map with zero operations leaves every element unchanged, which is
        // identical to copying the array. This is always an authoring mistake
        // (use the input directly), so we fail-fast here — consistent with
        // the sibling array nodes (filter/aggregate/sort/dedupe all require
        // their config fields to be non-trivial).
        if input.operations.is_empty() {
            return Err(ActionError::fatal(
                "map: at least one operation is required",
            ));
        }

        // ── 3. Apply operations to each element ───────────────────────────────
        //
        // Count-preserving: the output vec is pre-allocated to exactly the
        // same capacity as the input. Each element is consumed from `elements`
        // so we never clone the original values.
        let mut reshaped: Vec<Value> = Vec::with_capacity(elements.len());

        for element in elements {
            // Guard: every element must be a JSON object.
            //
            // `Value::get` on a non-object returns `None` silently, so Pick/Omit
            // would silently pass through non-objects (empty result) and Rename
            // would fire a "key not found" Fatal instead of the correct
            // "non-object element" Fatal. The explicit guard makes rejection
            // uniform across all operators.
            if !element.is_object() {
                return Err(ActionError::fatal(format!(
                    "map: every array element must be a JSON object, got {}",
                    element.type_name_str()
                )));
            }

            // Destructure the Value into its inner Map for in-place mutation.
            let Value::Object(mut fields) = element else {
                // Unreachable: the is_object() guard above guarantees this is
                // an Object variant. The else branch satisfies the exhaustiveness
                // check; no panic family is used.
                return Err(ActionError::fatal(
                    "map: internal invariant violated: element is not an object after guard",
                ));
            };

            apply_operations(&mut fields, &input.operations)?;
            reshaped.push(Value::Object(fields));
        }

        Ok(ActionResult::success(Value::Array(reshaped)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use nebula_action::{ActionError, ActionResult, StatelessAction};
    use serde_json::{Value, json};

    use crate::actions::json_transform::TransformOperation;

    use super::{MapAction, MapInput};

    fn run(input: MapInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
        let action = MapAction;
        let ctx = TestContextBuilder::new().build();
        async move { action.execute(input, &ctx).await }
    }

    fn extract_output(result: ActionResult<Value>) -> Value {
        result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .expect("ActionResult must carry a primary output value")
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

    // ── 1: non-array data is Fatal ────────────────────────────────────────────
    //
    // RED witness: without the type-guard arm the object would not be rejected
    // and `unwrap_err()` would panic.
    #[tokio::test]
    async fn non_array_data_is_fatal() {
        let input = MapInput {
            data: Some(json!({"a": 1})),
            operations: vec![pick(&["a"])],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for object data; got: {err:?}"
        );
    }

    // ── 2: null data is Fatal ─────────────────────────────────────────────────
    //
    // RED witness: without the Null arm, null data would not be rejected.
    #[tokio::test]
    async fn null_data_is_fatal() {
        let input = MapInput {
            data: Some(json!(null)),
            operations: vec![pick(&["a"])],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for null data; got: {err:?}"
        );
    }

    // ── 3: empty operations is Fatal ─────────────────────────────────────────
    //
    // A no-op map (zero operations) is always an authoring mistake.
    //
    // RED witness: without the `operations.is_empty()` guard, the action would
    // return the input array unchanged — no error, so `unwrap_err()` panics.
    #[tokio::test]
    async fn empty_operations_is_fatal() {
        let input = MapInput {
            data: Some(json!([{"a": 1}])),
            operations: vec![],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for empty operations; got: {err:?}"
        );
    }

    // ── 4: non-object element is Fatal ───────────────────────────────────────
    //
    // RED witness: without the `is_object()` guard, a scalar element would
    // silently yield an empty object (Pick on a non-object is a no-op via None)
    // instead of a Fatal error. `unwrap_err()` would panic on the Ok result.
    #[tokio::test]
    async fn non_object_element_is_fatal() {
        let input = MapInput {
            data: Some(json!([{"a": 1}, 42])),
            operations: vec![pick(&["a"])],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for non-object element; got: {err:?}"
        );
    }

    // ── 5: pick per element keeps only named keys ─────────────────────────────
    //
    // Input: [{a:1,b:2},{a:3,b:4}]  ops=[pick ["a"]]
    // Expected: [{a:1},{a:3}]
    //
    // RED witness: without the pick implementation, `b` would survive in the
    // output and the concrete assertion would fail.
    #[tokio::test]
    async fn map_pick_per_element() {
        let input = MapInput {
            data: Some(json!([{"a": 1, "b": 2}, {"a": 3, "b": 4}])),
            operations: vec![pick(&["a"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"a": 1}, {"a": 3}]),
            "pick must retain only 'a' on every element"
        );
    }

    // ── 6: rename per element moves the value to the new key ─────────────────
    //
    // Input: [{old:"x"},{old:"y"}]  ops=[rename old→new]
    // Expected: [{new:"x"},{new:"y"}]
    //
    // RED witness: without the rename, `old` remains and `new` is absent —
    // the concrete assertion fails.
    #[tokio::test]
    async fn map_rename_per_element() {
        let input = MapInput {
            data: Some(json!([{"old": "x"}, {"old": "y"}])),
            operations: vec![rename("old", "new")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"new": "x"}, {"new": "y"}]),
            "rename must move 'old' to 'new' on every element"
        );
    }

    // ── 7: omit per element removes the named key ─────────────────────────────
    //
    // Input: [{a:1,b:2},{a:3,b:4}]  ops=[omit ["b"]]
    // Expected: [{a:1},{a:3}]
    //
    // RED witness: without the omit, `b` survives in the output and the
    // concrete assertion fails.
    #[tokio::test]
    async fn map_omit_per_element() {
        let input = MapInput {
            data: Some(json!([{"a": 1, "b": 2}, {"a": 3, "b": 4}])),
            operations: vec![omit(&["b"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"a": 1}, {"a": 3}]),
            "omit must remove 'b' from every element"
        );
    }

    // ── 8: multi-op applied left-to-right per element ────────────────────────
    //
    // ops=[pick ["a","c"], rename "a"→"label"]
    // Input:  [{a:1,b:99,c:2},{a:3,b:88,c:4}]
    // Expected: [{label:1,c:2},{label:3,c:4}]
    //   — pick first removes `b`, then rename moves `a` → `label`.
    //
    // RED witness: reversing the op order (rename then pick) would lose "label"
    // because pick ["a","c"] would find only the original "a" name — the rename
    // would have moved it already, so pick drops it.
    #[tokio::test]
    async fn map_multi_op_per_element() {
        let input = MapInput {
            data: Some(json!([
                {"a": 1, "b": 99, "c": 2},
                {"a": 3, "b": 88, "c": 4}
            ])),
            operations: vec![pick(&["a", "c"]), rename("a", "label")],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"label": 1, "c": 2}, {"label": 3, "c": 4}]),
            "ops must be applied in declaration order: pick then rename"
        );
    }

    // ── 9: count preserved and original order maintained ─────────────────────
    //
    // Input: 4 elements in a non-sorted order. All survive (pick keeps "k").
    // Expected: same 4 elements in same order with only "k" remaining.
    //
    // RED witness: a sort-based impl would reorder; a filter-based impl would
    // drop some elements. The non-ascending order of "k" values (3,1,4,2)
    // ensures the order assertion catches any reordering.
    #[tokio::test]
    async fn map_preserves_count_and_order() {
        let input = MapInput {
            data: Some(json!([
                {"k": 3, "extra": "a"},
                {"k": 1, "extra": "b"},
                {"k": 4, "extra": "c"},
                {"k": 2, "extra": "d"}
            ])),
            operations: vec![pick(&["k"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"k": 3}, {"k": 1}, {"k": 4}, {"k": 2}]),
            "map must preserve element count and original order"
        );
    }

    // ── 10: rename with missing source on an element is Fatal ─────────────────
    //
    // The second element lacks the "src" field that the rename requires.
    //
    // RED witness: without error propagation from `apply_operations`, the action
    // would either return wrong data or panic. Propagating the Fatal makes this
    // test pass.
    #[tokio::test]
    async fn map_rename_missing_source_is_fatal() {
        let input = MapInput {
            data: Some(json!([
                {"src": "x"},   // first element: rename source present — ok
                {"other": "y"}  // second element: "src" absent — must be Fatal
            ])),
            operations: vec![rename("src", "dst")],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal when rename source key is absent on an element; got: {err:?}"
        );
    }

    // ── 11: empty input array → empty output array (not Fatal) ───────────────
    #[tokio::test]
    async fn map_empty_input_returns_empty_array() {
        let input = MapInput {
            data: Some(json!([])),
            operations: vec![pick(&["a"])],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([]),
            "empty input array must yield empty output array"
        );
    }

    // ── 12: action key is "core.map" ─────────────────────────────────────────
    #[test]
    fn action_key_is_core_dot_map() {
        use nebula_action::action::Action;
        assert_eq!(MapAction::metadata().base.key.as_str(), "core.map");
    }
}
