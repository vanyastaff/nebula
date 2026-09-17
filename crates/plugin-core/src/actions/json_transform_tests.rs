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
    let factory = nebula_action::GenericStatelessFactory::<JsonTransform>::new()
        .expect("JSON transform metadata must admit");
    assert_eq!(
        nebula_action::ActionFactory::metadata(&factory)
            .base()
            .key()
            .as_str(),
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
