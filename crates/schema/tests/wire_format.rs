//! Wire-format invariants — must not break at Phase 1.

use nebula_schema::{FieldValue, FieldValues};
use serde_json::json;

#[test]
fn plain_literal() {
    let v = FieldValue::from_json(json!("hello"));
    assert_eq!(v.to_json(), json!("hello"));
}

#[test]
fn expression_wrapper() {
    let src = json!({"$expr": "{{ $x.y }}"});
    let v = FieldValue::from_json(src.clone());
    assert_eq!(v.to_json(), src);
}

#[test]
fn mode_wrapper() {
    let src = json!({"mode": "oauth2", "value": {"scope": "read"}});
    let v = FieldValue::from_json(src.clone());
    assert_eq!(v.to_json(), src);
}

#[test]
fn nested_object_roundtrip() {
    let src = json!({
        "a": "x",
        "b": [1, {"k": true}],
        "c": {"$expr": "{{ $z }}"},
        "d": {"mode": "m"}
    });
    let values = FieldValues::from_json(src.clone()).unwrap();
    assert_eq!(values.to_json(), src);
}

#[test]
fn top_level_non_object_rejected() {
    let r = FieldValues::from_json(json!([1, 2]));
    assert!(r.is_err());
}
