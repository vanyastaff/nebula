use std::future::Future;

use nebula_action::testing::TestContextBuilder;
use serde_json::json;

use super::*;

fn run(input: ArrayInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
    let action = ArrayAction;
    let ctx = TestContextBuilder::new().build();
    async move { action.execute(input, &ctx).await }
}

fn extract_output(result: ActionResult<Value>) -> Value {
    result
        .into_primary_output()
        .and_then(nebula_action::ActionOutput::into_value)
        .expect("ActionResult must carry a primary output value")
}

// ── Chunk ─────────────────────────────────────────────────────────────────

// Even split: 4 elements / size 2 → two full chunks.
#[tokio::test]
async fn chunk_even_split() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3, 4])),
        operations: vec![ArrayOp::Chunk { size: 2 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([[1, 2], [3, 4]]));
}

// Uneven split: 5 elements / size 2 → last chunk is shorter.
#[tokio::test]
async fn chunk_uneven_last_chunk_shorter() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3, 4, 5])),
        operations: vec![ArrayOp::Chunk { size: 2 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([[1, 2], [3, 4], [5]]));
}

// RED witness: drop the `size == 0` guard in `apply_chunk` and this test
// fails — `slice::chunks(0)` panics, so the action would panic instead of
// returning Fatal.
#[tokio::test]
async fn chunk_size_zero_is_fatal() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3])),
        operations: vec![ArrayOp::Chunk { size: 0 }],
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for chunk size 0; got: {err:?}"
    );
}

// ── Flatten ───────────────────────────────────────────────────────────────

// One level: [[1,2],[3,4]] → [1,2,3,4].
#[tokio::test]
async fn flatten_one_level() {
    let input = ArrayInput {
        data: Some(json!([[1, 2], [3, 4]])),
        operations: vec![ArrayOp::Flatten { depth: 1 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, 2, 3, 4]));
}

// Deep: depth 2 merges two nesting levels; depth 1 would only merge one.
#[tokio::test]
async fn flatten_deep_depth_two() {
    let input = ArrayInput {
        data: Some(json!([[1, [2, 3]], [4, [5]]])),
        operations: vec![ArrayOp::Flatten { depth: 2 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, 2, 3, 4, 5]));
}

// depth 1 leaves the inner nesting intact (contrast with the deep test).
#[tokio::test]
async fn flatten_depth_one_leaves_inner_nesting() {
    let input = ArrayInput {
        data: Some(json!([[1, [2, 3]], [4, [5]]])),
        operations: vec![ArrayOp::Flatten { depth: 1 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, [2, 3], 4, [5]]));
}

// Non-array elements pass through unchanged at every level.
#[tokio::test]
async fn flatten_non_array_elements_pass_through() {
    let input = ArrayInput {
        data: Some(json!([1, [2, 3], "x", [4]])),
        operations: vec![ArrayOp::Flatten { depth: 1 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, 2, 3, "x", 4]));
}

// depth 0 is a no-op: the array passes through unchanged.
#[tokio::test]
async fn flatten_depth_zero_is_noop() {
    let input = ArrayInput {
        data: Some(json!([[1, 2], [3]])),
        operations: vec![ArrayOp::Flatten { depth: 0 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([[1, 2], [3]]));
}

// Default depth (omitted on the wire) is 1.
#[tokio::test]
async fn flatten_default_depth_is_one() {
    let op: ArrayOp = serde_json::from_value(json!({"op": "flatten"})).unwrap();
    let input = ArrayInput {
        data: Some(json!([[1], [2, 3]])),
        operations: vec![op],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, 2, 3]));
}

// ── Take ──────────────────────────────────────────────────────────────────

// count < len keeps the leading prefix.
#[tokio::test]
async fn take_count_less_than_len() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3, 4])),
        operations: vec![ArrayOp::Take { count: 2 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, 2]));
}

// count >= len keeps the whole array (saturating).
#[tokio::test]
async fn take_count_at_or_beyond_len_keeps_all() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3])),
        operations: vec![ArrayOp::Take { count: 10 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, 2, 3]));
}

// count == 0 produces an empty array.
#[tokio::test]
async fn take_count_zero_is_empty() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3])),
        operations: vec![ArrayOp::Take { count: 0 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([]));
}

// ── Skip ──────────────────────────────────────────────────────────────────

// count < len drops the leading prefix.
#[tokio::test]
async fn skip_count_less_than_len() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3, 4])),
        operations: vec![ArrayOp::Skip { count: 1 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([2, 3, 4]));
}

// count >= len produces an empty array (saturating, never panics on drain).
#[tokio::test]
async fn skip_count_at_or_beyond_len_is_empty() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3])),
        operations: vec![ArrayOp::Skip { count: 10 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([]));
}

// ── Composition ─────────────────────────────────────────────────────────────

// [skip 1, chunk 2]: drop the first element, then group the rest in pairs.
#[tokio::test]
async fn compose_skip_then_chunk() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3, 4, 5])),
        operations: vec![ArrayOp::Skip { count: 1 }, ArrayOp::Chunk { size: 2 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([[2, 3], [4, 5]]));
}

// Chunk then Flatten{1} round-trips back to the original flat array — proving
// ops compose on whatever the previous op produced (array-of-arrays here).
#[tokio::test]
async fn compose_chunk_then_flatten_round_trips() {
    let input = ArrayInput {
        data: Some(json!([1, 2, 3, 4, 5])),
        operations: vec![ArrayOp::Chunk { size: 2 }, ArrayOp::Flatten { depth: 1 }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([1, 2, 3, 4, 5]));
}

// ── Data / operations edge cases ────────────────────────────────────────────

// Empty operations list returns the data unchanged.
#[tokio::test]
async fn empty_operations_returns_data_unchanged() {
    let data = json!([1, 2, 3]);
    let input = ArrayInput {
        data: Some(data.clone()),
        operations: vec![],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, data);
}

// RED witness: drop the non-array guard arm and `unwrap_err()` panics on the
// Ok result. Non-array `data` must be Fatal.
#[tokio::test]
async fn non_array_data_is_fatal() {
    let input = ArrayInput {
        data: Some(json!({"a": 1})),
        operations: vec![],
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for object data; got: {err:?}"
    );
}

// null data is Fatal (consistent with the sibling array nodes).
#[tokio::test]
async fn null_data_is_fatal() {
    let input = ArrayInput {
        data: Some(Value::Null),
        operations: vec![],
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for null data; got: {err:?}"
    );
}

// ── Metadata ────────────────────────────────────────────────────────────────

#[test]
fn action_key_is_core_dot_array() {
    let factory = nebula_action::GenericStatelessFactory::<ArrayAction>::new()
        .expect("array metadata must admit");
    assert_eq!(
        nebula_action::ActionFactory::metadata(&factory)
            .base()
            .key()
            .as_str(),
        "core.array"
    );
}

// ── Serde round-trip for each ArrayOp wire shape ────────────────────────────

#[test]
fn serde_roundtrip_chunk() {
    let op = ArrayOp::Chunk { size: 2 };
    let wire = serde_json::to_value(&op).unwrap();
    assert_eq!(wire, json!({"op": "chunk", "size": 2}));
    let restored: ArrayOp = serde_json::from_value(wire).unwrap();
    assert_eq!(restored, op);
}

#[test]
fn serde_roundtrip_flatten() {
    let op = ArrayOp::Flatten { depth: 3 };
    let wire = serde_json::to_value(&op).unwrap();
    assert_eq!(wire, json!({"op": "flatten", "depth": 3}));
    let restored: ArrayOp = serde_json::from_value(wire).unwrap();
    assert_eq!(restored, op);
}

#[test]
fn serde_roundtrip_take() {
    let op = ArrayOp::Take { count: 5 };
    let wire = serde_json::to_value(&op).unwrap();
    assert_eq!(wire, json!({"op": "take", "count": 5}));
    let restored: ArrayOp = serde_json::from_value(wire).unwrap();
    assert_eq!(restored, op);
}

#[test]
fn serde_roundtrip_skip() {
    let op = ArrayOp::Skip { count: 1 };
    let wire = serde_json::to_value(&op).unwrap();
    assert_eq!(wire, json!({"op": "skip", "count": 1}));
    let restored: ArrayOp = serde_json::from_value(wire).unwrap();
    assert_eq!(restored, op);
}
