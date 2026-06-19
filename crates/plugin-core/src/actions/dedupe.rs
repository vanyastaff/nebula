//! `core.dedupe` — remove duplicate elements from a JSON array of objects,
//! keyed by one or more fields.
//!
//! Iterates the input array in order and keeps the **first** element for each
//! unique key-tuple; later elements with the same key-tuple are dropped. The
//! original order of kept elements is preserved.
//!
//! This fills a gap in the `{{ }}` expression language: `array.unique` dedupes
//! by whole-value equality and cannot dedupe by a named field subset. An empty
//! `keys` list that would replicate `array.unique` is rejected as redundant.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": [
//!     { "id": 1, "value": "a" },
//!     { "id": 2, "value": "b" },
//!     { "id": 1, "value": "c" }
//!   ],
//!   "keys": ["id"]
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! [
//!   { "id": 1, "value": "a" },
//!   { "id": 2, "value": "b" }
//! ]
//! ```
//!
//! (First occurrence of `id=1` is kept; the later duplicate is dropped.)
//!
//! ## Error semantics
//!
//! - `data` absent / null / non-array → **Fatal**.
//! - `keys` empty → **Fatal** (with a pointer to `array.unique` for whole-value
//!   dedup).
//! - Any array element that is not a JSON object → **Fatal** (explicit
//!   `is_object()` guard — `Value::get` on a non-object returns `None`
//!   silently, which would cause key reads to misfire).
//! - A `keys` field **absent** on an element → **Fatal** (cannot determine
//!   identity without the key; consistent with `core.aggregate`'s group_by
//!   missing-key rule). A `null` key value is allowed — null is a valid
//!   identity component.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::collections::HashSet;
use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::util::ValueTypeNameStr;

// ── Input types ───────────────────────────────────────────────────────────────

/// Input for `core.dedupe`.
///
/// `data` must be a JSON array of objects when present. `null` / absent values
/// are rejected with a Fatal error — deduping a non-array is always an
/// authoring mistake.
///
/// ## Wire shape
///
/// ```json
/// {
///   "data": [ { "id": 1, "v": "a" }, { "id": 1, "v": "b" } ],
///   "keys": ["id"]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupeInput {
    /// Array of JSON objects to deduplicate. Must be a JSON array when present.
    #[serde(default)]
    pub data: Option<Value>,
    /// Field names whose value-tuple defines element identity. At least one
    /// required; empty `keys` is rejected — use `array.unique` for
    /// whole-value dedup.
    pub keys: Vec<String>,
}

// `data` is fully dynamic; the module doc describes expected structure.
impl HasSchema for DedupeInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that removes duplicate elements from a JSON array of objects
/// by one or more key fields. First occurrence wins; original order preserved.
///
/// Keyed `core.dedupe`. No I/O, no credentials, no resources.
///
/// ## Example wire input / output
///
/// ```json
/// {
///   "data": [
///     { "user": "alice", "event": "login"  },
///     { "user": "bob",   "event": "login"  },
///     { "user": "alice", "event": "logout" }
///   ],
///   "keys": ["user"]
/// }
/// ```
///
/// Output:
/// ```json
/// [
///   { "user": "alice", "event": "login" },
///   { "user": "bob",   "event": "login" }
/// ]
/// ```
#[derive(Debug)]
pub struct Dedupe;

impl nebula_action::action::Action for Dedupe {
    type Input = DedupeInput;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.dedupe"),
            "Dedupe",
            "Remove duplicate array elements by one or more key fields (first occurrence wins)",
        )
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for Dedupe {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Dedupe)
    }
}

impl StatelessAction for Dedupe {
    #[instrument(name = "core.dedupe", skip_all, fields(element_count))]
    async fn execute(
        &self,
        input: DedupeInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // ── 1. Validate data ──────────────────────────────────────────────────
        let elements: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "dedupe: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "dedupe: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        tracing::Span::current().record("element_count", elements.len());

        // ── 2. Validate keys non-empty ────────────────────────────────────────
        if input.keys.is_empty() {
            return Err(ActionError::fatal(
                "dedupe: at least one key field is required \
                 (use the array.unique expression for whole-value dedup)",
            ));
        }

        // ── 3. Iterate in order; keep first-seen key-tuples ───────────────────
        //
        // `seen_key_tuples` tracks serialized identity tuples so duplicates are
        // detected in O(1) per element. serde_json's `Value::Number` stores
        // integer and float representations separately, so serialization keeps
        // `1`, `"1"`, and `1.0` distinct. Object key-fields serialize with their
        // keys in sorted order (serde_json's default `BTreeMap`-backed `Map`; the
        // `preserve_order` feature is not enabled), so `{"a":1,"b":2}` and
        // `{"b":2,"a":1}` are the SAME identity; array key-fields keep element
        // order, so `[1,2]` and `[2,1]` are DISTINCT identities.
        let mut seen_key_tuples: HashSet<String> = HashSet::new();
        let mut kept_elements: Vec<Value> = Vec::new();

        for element in elements {
            // Guard: every element must be a JSON object.
            // `Value::get` on a non-object returns `None` silently, so key reads
            // would misfire without this explicit check.
            if !element.is_object() {
                return Err(ActionError::fatal(format!(
                    "dedupe: every array element must be a JSON object, got {}",
                    element.type_name_str()
                )));
            }

            // Build the canonical identity tuple for this element.
            //
            // A field that is ABSENT is Fatal (can't determine identity).
            // A field that is present but has a `null` value is allowed —
            // null is a valid identity component.
            let mut key_values: Vec<Value> = Vec::with_capacity(input.keys.len());
            for key_field in &input.keys {
                match element.get(key_field.as_str()) {
                    Some(field_value) => key_values.push(field_value.clone()),
                    None => {
                        return Err(ActionError::fatal(format!(
                            "dedupe: key field `{key_field}` missing on an element"
                        )));
                    },
                }
            }

            // Serialize the key-tuple to a canonical string for HashSet membership.
            // `key_values` contains only cloned JSON Values — serialization should
            // not fail, but we propagate any error rather than panic.
            let serialized_tuple = serde_json::to_string(&Value::Array(key_values))
                .map_err(|e| ActionError::fatal(format!("dedupe: failed to serialize key: {e}")))?;

            // First-occurrence wins: insert returns false when the key was already
            // present, indicating a duplicate that should be dropped.
            if seen_key_tuples.insert(serialized_tuple) {
                kept_elements.push(element);
            }
        }

        Ok(ActionResult::success(Value::Array(kept_elements)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use nebula_action::{ActionError, ActionResult, StatelessAction};
    use serde_json::{Value, json};

    use super::{Dedupe, DedupeInput};

    fn run(input: DedupeInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
        let action = Dedupe;
        let ctx = TestContextBuilder::new().build();
        async move { action.execute(input, &ctx).await }
    }

    fn extract_output(result: ActionResult<Value>) -> Value {
        result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .expect("ActionResult must carry a primary output value")
    }

    // ── 1: non-array data is Fatal ────────────────────────────────────────────
    //
    // RED witness: without the type-guard arm, the object would not be rejected
    // and `unwrap_err()` would panic.
    #[tokio::test]
    async fn non_array_data_is_fatal() {
        let input = DedupeInput {
            data: Some(json!({"id": 1})),
            keys: vec!["id".into()],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for object data; got: {err:?}"
        );
    }

    // ── 2: null data is Fatal ─────────────────────────────────────────────────
    #[tokio::test]
    async fn null_data_is_fatal() {
        let input = DedupeInput {
            data: Some(json!(null)),
            keys: vec!["id".into()],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for null data; got: {err:?}"
        );
    }

    // ── 2b: absent data is Fatal (distinct match path from `Some(Null)`) ──────
    #[tokio::test]
    async fn absent_data_is_fatal() {
        let input = DedupeInput {
            data: None,
            keys: vec!["id".into()],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for absent data; got: {err:?}"
        );
    }

    // ── 2c: composite (array) key value is order-sensitive ────────────────────
    //
    // A key field whose value is itself an array is compared by canonical JSON
    // serialization, which preserves array order — so `["a","b"]` and `["b","a"]`
    // are DISTINCT identities, while a repeated `["a","b"]` is deduped.
    #[tokio::test]
    async fn composite_array_key_is_order_sensitive() {
        let input = DedupeInput {
            data: Some(json!([
                {"tags": ["a", "b"]},
                {"tags": ["b", "a"]},
                {"tags": ["a", "b"]}
            ])),
            keys: vec!["tags".into()],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"tags": ["a", "b"]}, {"tags": ["b", "a"]}]),
            "array key values are order-sensitive: [a,b] != [b,a]; the repeated [a,b] is dropped"
        );
    }

    // ── 3: empty keys is Fatal ────────────────────────────────────────────────
    //
    // RED witness: without the `keys.is_empty()` guard, iterating would
    // serialize every element's key-tuple as "[]" — all elements would appear
    // as the same key, so only the first element would survive. That is not an
    // error, but it IS wrong (and redundant with array.unique). The Fatal here
    // points the author to the right builtin.
    #[tokio::test]
    async fn empty_keys_is_fatal() {
        let input = DedupeInput {
            data: Some(json!([{"id": 1}])),
            keys: vec![],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for empty keys; got: {err:?}"
        );
    }

    // ── 4: non-object element is Fatal ────────────────────────────────────────
    //
    // `Value::get` on a non-object returns `None` silently. Without the
    // explicit `is_object()` guard, key reads would produce a "missing key"
    // Fatal instead of the more precise "non-object element" Fatal — but we
    // want the right error, not just any error.
    //
    // RED witness: without the `is_object()` guard, the number `5` would not
    // reach the missing-key check (there are no keys to iterate when the guard
    // is absent in the wrong way). Prove the specific guard exists by asserting
    // Fatal.
    #[tokio::test]
    async fn non_object_element_is_fatal() {
        let input = DedupeInput {
            data: Some(json!([{"id": 1}, 5])),
            keys: vec!["id".into()],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for non-object element; got: {err:?}"
        );
    }

    // ── 5: missing key field is Fatal ────────────────────────────────────────
    //
    // An absent key field means we cannot determine identity. Fail-closed is
    // correct; a null value in the field IS allowed (see test 6 below).
    //
    // RED witness: without the `None` arm returning Fatal, the loop would
    // silently skip the field (or produce a wrong key), causing `unwrap_err()`
    // to panic on the Ok result.
    #[tokio::test]
    async fn missing_key_field_is_fatal() {
        let input = DedupeInput {
            // Second element is missing the "id" field entirely.
            data: Some(json!([{"id": 1}, {"value": "x"}])),
            keys: vec!["id".into()],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for missing key field; got: {err:?}"
        );
    }

    // ── 6: dedupe single key keeps first occurrence ───────────────────────────
    //
    // Input:  [{id:1,v:"a"},{id:2,v:"b"},{id:1,v:"c"}]  keys=["id"]
    // Expected: [{id:1,v:"a"},{id:2,v:"b"}]
    //   — first id=1 element (v="a") is kept; the later id=1 element (v="c")
    //     is dropped. id=2 is unique, kept.
    //
    // RED witness: keeping the LAST occurrence would produce
    // [{id:1,v:"c"},{id:2,v:"b"}], failing the concrete assertion.
    // Not deduplicating at all would produce all three elements.
    #[tokio::test]
    async fn dedupe_single_key_keeps_first_occurrence() {
        let input = DedupeInput {
            data: Some(json!([
                {"id": 1, "v": "a"},
                {"id": 2, "v": "b"},
                {"id": 1, "v": "c"}
            ])),
            keys: vec!["id".into()],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"id": 1, "v": "a"}, {"id": 2, "v": "b"}]),
            "first id=1 (v=a) must be kept; second id=1 (v=c) must be dropped"
        );
    }

    // ── 7: dedupe multi-key ───────────────────────────────────────────────────
    //
    // keys=["a","b"]: two elements with the same `a` but different `b` are
    // BOTH kept (distinct identity tuples). Two elements with the same (a,b)
    // pair are deduped to the first.
    //
    // RED witness: deduping only by the first key would drop the second element
    // (same a=1, different b), causing the assertion to fail.
    #[tokio::test]
    async fn dedupe_multi_key() {
        let input = DedupeInput {
            data: Some(json!([
                {"a": 1, "b": 10, "marker": "first"},
                {"a": 1, "b": 20, "marker": "second"},  // different b → kept
                {"a": 1, "b": 10, "marker": "third"}    // same (a,b) → dropped
            ])),
            keys: vec!["a".into(), "b".into()],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([
                {"a": 1, "b": 10, "marker": "first"},
                {"a": 1, "b": 20, "marker": "second"}
            ]),
            "different b must produce distinct identity tuples; \
             same (a,b) must deduplicate to first occurrence"
        );
    }

    // ── 8: dedupe preserves order with interleaved duplicates ─────────────────
    //
    // Input:  [{k:1},{k:2},{k:1},{k:3}]  keys=["k"]
    // Expected: [{k:1},{k:2},{k:3}]  — original order of first occurrences.
    //
    // RED witness: re-ordering kept elements (e.g. sorted by key) would produce
    // [{k:1},{k:2},{k:3}] with different input order — but an incorrectly
    // sorted impl on a different input could accidentally pass. The interleaved
    // input [1,2,1,3] stresses that k=2 and k=3 appear in insertion order,
    // not in a reordered form.
    #[tokio::test]
    async fn dedupe_preserves_order_with_interleaved_dupes() {
        let input = DedupeInput {
            data: Some(json!([{"k": 1}, {"k": 2}, {"k": 1}, {"k": 3}])),
            keys: vec!["k".into()],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"k": 1}, {"k": 2}, {"k": 3}]),
            "first occurrences must appear in their original input order"
        );
    }

    // ── 9: all unique elements pass through unchanged ─────────────────────────
    //
    // RED witness: a buggy impl that always drops the second element (regardless
    // of key uniqueness) would fail this test for arrays with more than one
    // element.
    #[tokio::test]
    async fn dedupe_all_unique_passthrough() {
        let input = DedupeInput {
            data: Some(json!([{"k": 1}, {"k": 2}, {"k": 3}])),
            keys: vec!["k".into()],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"k": 1}, {"k": 2}, {"k": 3}]),
            "all-unique input must pass through unchanged"
        );
    }

    // ── 10: null key value is allowed; two nulls dedupe ───────────────────────
    //
    // A present field whose value is `null` is a valid identity component —
    // only ABSENT keys are Fatal. Two elements with `null` on the key field
    // share the same identity tuple ([null]) and the second is dropped.
    //
    // RED witness: treating null as "skip this element" or as Fatal would
    // either leave two null elements or return an error, both failing the
    // assertion.
    #[tokio::test]
    async fn dedupe_null_key_value_is_allowed() {
        let input = DedupeInput {
            data: Some(json!([{"k": null}, {"k": null}, {"k": 1}])),
            keys: vec!["k".into()],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"k": null}, {"k": 1}]),
            "null is a valid key value; two nulls must dedupe to the first occurrence"
        );
    }

    // ── 11: empty input returns empty array ───────────────────────────────────
    #[tokio::test]
    async fn empty_input_returns_empty_array() {
        let input = DedupeInput {
            data: Some(json!([])),
            keys: vec!["id".into()],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([]), "empty input must return empty array");
    }

    // ── 12: action key is "core.dedupe" ──────────────────────────────────────
    #[test]
    fn action_key_is_core_dot_dedupe() {
        use nebula_action::action::Action;
        assert_eq!(Dedupe::metadata().base.key.as_str(), "core.dedupe");
    }
}
