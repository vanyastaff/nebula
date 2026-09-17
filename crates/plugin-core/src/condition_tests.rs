use nebula_action::ActionError;
use serde_json::json;

use super::*;

// ── normalize_data ────────────────────────────────────────────────────────

#[test]
fn normalize_none_yields_empty_object() {
    let result = normalize_data(None).unwrap();
    assert_eq!(result, json!({}));
}

#[test]
fn normalize_null_yields_empty_object() {
    let result = normalize_data(Some(json!(null))).unwrap();
    assert_eq!(result, json!({}));
}

#[test]
fn normalize_object_passes_through() {
    let obj = json!({"a": 1});
    let result = normalize_data(Some(obj.clone())).unwrap();
    assert_eq!(result, obj);
}

#[test]
fn normalize_array_returns_fatal() {
    let err = normalize_data(Some(json!([1, 2]))).unwrap_err();
    assert!(matches!(err, ActionError::Fatal { .. }));
}

#[test]
fn normalize_string_returns_fatal() {
    let err = normalize_data(Some(json!("hello"))).unwrap_err();
    assert!(matches!(err, ActionError::Fatal { .. }));
}

#[test]
fn normalize_number_returns_fatal() {
    let err = normalize_data(Some(json!(42))).unwrap_err();
    assert!(matches!(err, ActionError::Fatal { .. }));
}

#[test]
fn normalize_bool_returns_fatal() {
    let err = normalize_data(Some(json!(true))).unwrap_err();
    assert!(matches!(err, ActionError::Fatal { .. }));
}

// ── is_truthy ─────────────────────────────────────────────────────────────

#[test]
fn truthy_none_is_false() {
    assert!(!is_truthy(None));
}

#[test]
fn truthy_false_bool_is_false() {
    assert!(!is_truthy(Some(&json!(false))));
}

#[test]
fn truthy_true_bool_is_true() {
    assert!(is_truthy(Some(&json!(true))));
}

#[test]
fn truthy_null_is_false() {
    assert!(!is_truthy(Some(&json!(null))));
}

#[test]
fn truthy_zero_int_is_false() {
    assert!(!is_truthy(Some(&json!(0))));
}

#[test]
fn truthy_zero_float_is_false() {
    assert!(!is_truthy(Some(&json!(0.0))));
}

#[test]
fn truthy_nonzero_int_is_true() {
    assert!(is_truthy(Some(&json!(42))));
}

#[test]
fn truthy_empty_string_is_false() {
    assert!(!is_truthy(Some(&json!(""))));
}

#[test]
fn truthy_non_empty_string_is_true() {
    assert!(is_truthy(Some(&json!("hi"))));
}

#[test]
fn truthy_empty_array_is_false() {
    assert!(!is_truthy(Some(&json!([]))));
}

#[test]
fn truthy_non_empty_array_is_true() {
    assert!(is_truthy(Some(&json!([1]))));
}

#[test]
fn truthy_empty_object_is_false() {
    assert!(!is_truthy(Some(&json!({}))));
}

#[test]
fn truthy_non_empty_object_is_true() {
    assert!(is_truthy(Some(&json!({"k": 1}))));
}

// ── evaluate_condition — Eq ───────────────────────────────────────────────

#[test]
fn eq_matching_field_is_true() {
    let data = json!({"status": "active"});
    let cond = Condition::Leaf {
        field: "status".into(),
        op: ConditionOp::Eq,
        value: Some(json!("active")),
    };
    assert!(evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn eq_non_matching_field_is_false() {
    let data = json!({"status": "inactive"});
    let cond = Condition::Leaf {
        field: "status".into(),
        op: ConditionOp::Eq,
        value: Some(json!("active")),
    };
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn eq_missing_field_is_false() {
    let data = json!({"other": 1});
    let cond = Condition::Leaf {
        field: "status".into(),
        op: ConditionOp::Eq,
        value: Some(json!("active")),
    };
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

// RED witness: absent `value` for Eq must Fatal, not silently compare as null.
#[test]
fn eq_missing_value_returns_fatal() {
    let data = json!({"x": "hello"});
    let cond = Condition::Leaf {
        field: "x".into(),
        op: ConditionOp::Eq,
        value: None,
    };
    let err = evaluate_condition(&data, &cond).unwrap_err();
    assert!(matches!(err, ActionError::Fatal { .. }));
}

// ── evaluate_condition — Ne ───────────────────────────────────────────────

#[test]
fn ne_different_is_true() {
    let data = json!({"status": "inactive"});
    let cond = Condition::Leaf {
        field: "status".into(),
        op: ConditionOp::Ne,
        value: Some(json!("active")),
    };
    assert!(evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn ne_same_is_false() {
    let data = json!({"status": "active"});
    let cond = Condition::Leaf {
        field: "status".into(),
        op: ConditionOp::Ne,
        value: Some(json!("active")),
    };
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn ne_missing_field_is_true() {
    let data = json!({});
    let cond = Condition::Leaf {
        field: "status".into(),
        op: ConditionOp::Ne,
        value: Some(json!("active")),
    };
    assert!(evaluate_condition(&data, &cond).unwrap());
}

// ── evaluate_condition — Gt / ordered ────────────────────────────────────

#[test]
fn gt_numbers_true() {
    let data = json!({"score": 10});
    let cond = Condition::Leaf {
        field: "score".into(),
        op: ConditionOp::Gt,
        value: Some(json!(5)),
    };
    assert!(evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn gt_numbers_false() {
    let data = json!({"score": 3});
    let cond = Condition::Leaf {
        field: "score".into(),
        op: ConditionOp::Gt,
        value: Some(json!(5)),
    };
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

/// Large integers (> 2^53) compare EXACTLY — they must not collapse to
/// Equal via an f64 round-trip (Snowflake / 64-bit DB IDs).
///
/// RED witness: with the old f64-only path both values became the same f64,
/// so `compare_ordered` returned `Equal` and this `gt` was false.
#[test]
fn gt_large_integers_beyond_f64_precision() {
    // 2^53 and 2^53 + 1 are distinct i64 but round to the SAME f64.
    let data = json!({ "id": 9_007_199_254_740_993_i64 });
    let cond = Condition::Leaf {
        field: "id".into(),
        op: ConditionOp::Gt,
        value: Some(json!(9_007_199_254_740_992_i64)),
    };
    assert!(
        evaluate_condition(&data, &cond).unwrap(),
        "2^53+1 must be strictly greater than 2^53"
    );
}

#[test]
fn compare_ordered_large_integers_exact() {
    use std::cmp::Ordering;
    let bigger = json!(9_007_199_254_740_993_i64);
    let smaller = json!(9_007_199_254_740_992_i64);
    assert_eq!(
        compare_ordered(&bigger, &smaller).unwrap(),
        Ordering::Greater
    );
    assert_eq!(compare_ordered(&smaller, &bigger).unwrap(), Ordering::Less);
    // Genuine floats still compare via f64.
    assert_eq!(
        compare_ordered(&json!(1.5), &json!(2.5)).unwrap(),
        Ordering::Less
    );
}

// RED witness: type mismatch must Fatal, not silently return false.
#[test]
fn gt_type_mismatch_returns_fatal() {
    let data = json!({"score": 10});
    let cond = Condition::Leaf {
        field: "score".into(),
        op: ConditionOp::Gt,
        value: Some(json!("five")),
    };
    let err = evaluate_condition(&data, &cond).unwrap_err();
    assert!(matches!(err, ActionError::Fatal { .. }));
}

// RED witness: missing field on ordered op must Fatal, not return false.
#[test]
fn gt_missing_field_returns_fatal() {
    let data = json!({});
    let cond = Condition::Leaf {
        field: "score".into(),
        op: ConditionOp::Gt,
        value: Some(json!(5)),
    };
    let err = evaluate_condition(&data, &cond).unwrap_err();
    assert!(matches!(err, ActionError::Fatal { .. }));
}

// ── evaluate_condition — Exists / NotExists ───────────────────────────────

#[test]
fn exists_present_is_true() {
    let data = json!({"key": null});
    let cond = Condition::Leaf {
        field: "key".into(),
        op: ConditionOp::Exists,
        value: None,
    };
    assert!(evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn exists_absent_is_false() {
    let data = json!({});
    let cond = Condition::Leaf {
        field: "missing".into(),
        op: ConditionOp::Exists,
        value: None,
    };
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn not_exists_absent_is_true() {
    let data = json!({});
    let cond = Condition::Leaf {
        field: "missing".into(),
        op: ConditionOp::NotExists,
        value: None,
    };
    assert!(evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn not_exists_present_is_false() {
    let data = json!({"key": "val"});
    let cond = Condition::Leaf {
        field: "key".into(),
        op: ConditionOp::NotExists,
        value: None,
    };
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

// ── ConditionOp serde ────────────────────────────────────────────────────

#[test]
fn condition_op_serde_roundtrip() {
    for op in [
        ConditionOp::Eq,
        ConditionOp::Ne,
        ConditionOp::Gt,
        ConditionOp::Gte,
        ConditionOp::Lt,
        ConditionOp::Lte,
        ConditionOp::Exists,
        ConditionOp::NotExists,
        ConditionOp::Truthy,
    ] {
        let serialized = serde_json::to_string(&op).unwrap();
        let round_tripped: ConditionOp = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            round_tripped, op,
            "ConditionOp::{op:?} must survive a serde round-trip"
        );
    }
}

// ── Combinator: All ───────────────────────────────────────────────────────

fn leaf_exists(field: &str) -> Condition {
    Condition::Leaf {
        field: field.into(),
        op: ConditionOp::Exists,
        value: None,
    }
}

fn leaf_gt(field: &str, threshold: i64) -> Condition {
    Condition::Leaf {
        field: field.into(),
        op: ConditionOp::Gt,
        value: Some(json!(threshold)),
    }
}

fn leaf_eq(field: &str, val: &str) -> Condition {
    Condition::Leaf {
        field: field.into(),
        op: ConditionOp::Eq,
        value: Some(json!(val)),
    }
}

#[test]
fn all_all_true_is_true() {
    let data = json!({"a": 1, "b": "hello"});
    let cond = Condition::All(vec![leaf_exists("a"), leaf_exists("b")]);
    assert!(evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn all_one_false_is_false() {
    let data = json!({"a": 1});
    // "b" does not exist → leaf_exists("b") is false → All is false
    let cond = Condition::All(vec![leaf_exists("a"), leaf_exists("b")]);
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn all_empty_is_true() {
    let data = json!({});
    let cond = Condition::All(vec![]);
    assert!(evaluate_condition(&data, &cond).unwrap());
}

// ── Combinator: Any ───────────────────────────────────────────────────────

#[test]
fn any_one_true_is_true() {
    let data = json!({"b": 5});
    // "a" missing (false), "b" exists (true)
    let cond = Condition::Any(vec![leaf_exists("a"), leaf_exists("b")]);
    assert!(evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn any_all_false_is_false() {
    let data = json!({});
    let cond = Condition::Any(vec![leaf_exists("a"), leaf_exists("b")]);
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn any_empty_is_false() {
    let data = json!({"x": 1});
    let cond = Condition::Any(vec![]);
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

// ── Combinator: Not ───────────────────────────────────────────────────────

#[test]
fn not_of_true_is_false() {
    let data = json!({"x": 1});
    let cond = Condition::Not(Box::new(leaf_exists("x")));
    assert!(!evaluate_condition(&data, &cond).unwrap());
}

#[test]
fn not_of_false_is_true() {
    let data = json!({});
    let cond = Condition::Not(Box::new(leaf_exists("x")));
    assert!(evaluate_condition(&data, &cond).unwrap());
}

// ── Combinator: nesting ───────────────────────────────────────────────────

#[test]
fn all_of_any_nested_is_true() {
    // All([ Any([exists("a"), eq("status","active")]), gt("score", 0) ])
    // data has: status=active, score=5
    // Any: eq(status,active) = true → Any = true
    // gt(score,0) = true
    // All = true
    let data = json!({"status": "active", "score": 5});
    let cond = Condition::All(vec![
        Condition::Any(vec![leaf_exists("a"), leaf_eq("status", "active")]),
        leaf_gt("score", 0),
    ]);
    assert!(evaluate_condition(&data, &cond).unwrap());
}

// ── Fatal propagation inside combinators ──────────────────────────────────

// A `gt` on a missing field inside All must propagate Fatal immediately.
// The Fatal child is placed first so All evaluates it before any short-circuit.
// RED: if All swallowed the error and returned Ok(false), this would not unwrap_err().
#[test]
fn fatal_in_all_child_propagates() {
    let data = json!({}); // "score" missing → gt Fatal
    // leaf_gt("score", 0) is the first child; All evaluates it immediately → Fatal.
    let cond = Condition::All(vec![leaf_gt("score", 0), leaf_exists("anything")]);
    let err = evaluate_condition(&data, &cond).unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal from All child; got: {err:?}"
    );
}

#[test]
fn fatal_in_any_child_propagates() {
    let data = json!({}); // both "a" missing (Exists→false) and "score" missing (Gt→Fatal)
    // Any evaluates "a" first → false, then "score" → Fatal
    let cond = Condition::Any(vec![leaf_exists("a"), leaf_gt("score", 0)]);
    let err = evaluate_condition(&data, &cond).unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal from Any child; got: {err:?}"
    );
}

#[test]
fn not_of_fatal_propagates() {
    let data = json!({}); // "score" missing → Gt Fatal
    let cond = Condition::Not(Box::new(leaf_gt("score", 0)));
    let err = evaluate_condition(&data, &cond).unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal from Not child; got: {err:?}"
    );
}

// ── Serde: leaf round-trip ────────────────────────────────────────────────

// RED: under `#[serde(untagged)]` a bad op gives "data did not match any variant";
// with custom deserialization it gives the ConditionOp variant error containing "bogus".
#[test]
fn leaf_bad_op_gives_actionable_error() {
    let result = serde_json::from_str::<Condition>(r#"{"field":"x","op":"bogus","value":1}"#);
    let err = result.expect_err("must fail to deserialize unknown op");
    let msg = err.to_string();
    assert!(
        msg.contains("bogus") || msg.contains("unknown variant"),
        "error message must name the bad value or say 'unknown variant'; got: {msg}"
    );
}

// ── Strict wire-shape decoding ───────────────────────────────────────
#[test]
fn mixed_leaf_and_combinator_shape_is_rejected() {
    let result = serde_json::from_str::<Condition>(
        r#"{"field":"status","op":"eq","value":"active","all":"metadata"}"#,
    );
    assert!(
        result
            .expect_err("mixed condition shapes must fail closed")
            .to_string()
            .contains("exactly one shape"),
        "the error must identify the shape ambiguity"
    );
}

// A pure combinator shape (no "field" key) with a non-array "all" value
// must produce a clean error — not a panic.
#[test]
fn combinator_all_with_non_array_value_errors() {
    let result = serde_json::from_str::<Condition>(r#"{"all":"nope"}"#);
    assert!(
        result.is_err(),
        r#"{{"all":"nope"}} must fail (expected Vec<Condition>, got string)"#
    );
}

// A real All (no "field" key) must still deserialize as Condition::All.
#[test]
fn real_all_without_field_key_parses_as_all() {
    let result = serde_json::from_str::<Condition>(
        r#"{"all":[{"field":"a","op":"exists"},{"field":"b","op":"exists"}]}"#,
    );
    let cond = result.expect("must parse as All");
    assert!(
        matches!(cond, Condition::All(ref cs) if cs.len() == 2),
        "must be Condition::All with 2 children; got: {cond:?}"
    );
}

// ── Ambiguous combinator detection ────────────────────────────────────────

// RED with the old first-match short-circuit: `{"all":[],"any":[]}` would
// have returned `Ok(Condition::All([]))` silently ignoring `"any"`.
// With the ambiguity check it must return an Err whose message names the
// problem.
#[test]
fn ambiguous_combinator_keys_error() {
    let result = serde_json::from_str::<Condition>(r#"{"all":[],"any":[]}"#);
    let err = result.expect_err("ambiguous combinator must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("ambiguous") || msg.contains("at most one"),
        "error must describe the ambiguity; got: {msg}"
    );
}

// A single `any` combinator (no `field`) must still parse as Any.
#[test]
fn single_any_combinator_parses_as_any() {
    let cond = serde_json::from_str::<Condition>(r#"{"any":[{"field":"flag","op":"truthy"}]}"#)
        .expect("single-combinator Any must parse");
    assert!(
        matches!(cond, Condition::Any(ref cs) if cs.len() == 1),
        "must be Condition::Any with 1 child; got: {cond:?}"
    );
}

// A single `not` combinator (no `field`) must still parse as Not.
#[test]
fn single_not_combinator_parses_as_not() {
    let cond = serde_json::from_str::<Condition>(r#"{"not":{"field":"archived","op":"truthy"}}"#)
        .expect("single-combinator Not must parse");
    assert!(
        matches!(cond, Condition::Not(_)),
        "must be Condition::Not; got: {cond:?}"
    );
}

#[test]
fn leaf_with_unknown_field_is_rejected() {
    let result = serde_json::from_str::<Condition>(
        r#"{"field":"status","op":"eq","value":"active","metadata":"x"}"#,
    );
    assert!(
        result
            .expect_err("unknown leaf fields must fail closed")
            .to_string()
            .contains("unknown field"),
        "the error must identify the unknown field"
    );
}

#[test]
fn combinator_with_unknown_field_is_rejected() {
    let result = serde_json::from_str::<Condition>(r#"{"all":[],"metadata":"x"}"#);
    assert!(
        result
            .expect_err("unknown combinator fields must fail closed")
            .to_string()
            .contains("unknown field"),
        "the error must identify the unknown field"
    );
}

#[test]
fn leaf_serde_roundtrip() {
    let cond = Condition::Leaf {
        field: "status".into(),
        op: ConditionOp::Eq,
        value: Some(json!("active")),
    };
    let serialized = serde_json::to_value(&cond).unwrap();
    // Must serialize to the FLAT form — no wrapper key.
    assert_eq!(
        serialized,
        json!({"field": "status", "op": "eq", "value": "active"}),
        "Leaf must serialize to flat object without wrapper key"
    );
    let restored: Condition = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored, cond, "Leaf must survive a serde round-trip");
}

#[test]
fn leaf_no_value_serde_roundtrip() {
    let cond = Condition::Leaf {
        field: "key".into(),
        op: ConditionOp::Exists,
        value: None,
    };
    let serialized = serde_json::to_value(&cond).unwrap();
    // `value` must be absent when None.
    assert_eq!(
        serialized,
        json!({"field": "key", "op": "exists"}),
        "Leaf with None value must serialize without 'value' key"
    );
    let restored: Condition = serde_json::from_value(serialized).unwrap();
    assert_eq!(
        restored, cond,
        "Leaf (no value) must survive a serde round-trip"
    );
}

#[test]
fn all_serde_roundtrip() {
    let cond = Condition::All(vec![leaf_exists("a"), leaf_exists("b")]);
    let serialized = serde_json::to_value(&cond).unwrap();
    assert_eq!(serialized["all"].as_array().unwrap().len(), 2);
    let restored: Condition = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored, cond);
}

#[test]
fn any_serde_roundtrip() {
    let cond = Condition::Any(vec![leaf_eq("status", "active")]);
    let serialized = serde_json::to_value(&cond).unwrap();
    assert!(serialized.get("any").is_some(), "must have 'any' key");
    let restored: Condition = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored, cond);
}

#[test]
fn not_serde_roundtrip() {
    let cond = Condition::Not(Box::new(leaf_eq("archived", "true")));
    let serialized = serde_json::to_value(&cond).unwrap();
    assert!(serialized.get("not").is_some(), "must have 'not' key");
    let restored: Condition = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored, cond);
}
