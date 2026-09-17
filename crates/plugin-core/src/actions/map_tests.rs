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
    // The shared `apply_operations` must name the CALLING action: a rename
    // failure inside `core.map` must say `map:`, never `json_transform:`.
    let message = std::error::Error::source(&err)
        .map(ToString::to_string)
        .expect("fatal map error must retain its source");
    assert!(
        message.contains("map:") && !message.contains("json_transform"),
        "rename-missing error must be prefixed `map:`, not `json_transform:`; got: {message}"
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
    let factory = nebula_action::GenericStatelessFactory::<MapAction>::new()
        .expect("map metadata must admit");
    assert_eq!(
        nebula_action::ActionFactory::metadata(&factory)
            .base()
            .key()
            .as_str(),
        "core.map"
    );
}
