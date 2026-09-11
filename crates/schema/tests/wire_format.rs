//! Explicit authoring shorthand and schema-root shape boundaries.

use nebula_schema::{AuthoredValue, Schema, ValidSchema};
use serde_json::json;

#[test]
fn plain_literal() {
    let v = AuthoredValue::from_template_json(json!("hello")).unwrap();
    assert_eq!(v.to_json(), json!("hello"));
}

#[test]
fn expression_wrapper() {
    let src = json!({"$expr": "{{ $x.y }}"});
    let v = AuthoredValue::from_template_json(src.clone()).unwrap();
    assert_eq!(v.to_json(), src);
}

#[test]
fn mode_wrapper() {
    let src = json!({"mode": "oauth2", "value": {"scope": "read"}});
    let v = AuthoredValue::from_template_json(src.clone()).unwrap();
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
    let values = AuthoredValue::from_template_json(src.clone()).unwrap();
    assert_eq!(values.to_json(), src);
}

#[test]
fn root_shape_is_checked_by_the_schema_not_the_data_container() {
    let values = AuthoredValue::from_template_json(json!([1, 2])).unwrap();
    let record = Schema::builder().build().unwrap();
    let report = record.validate(values.clone()).unwrap_err();
    assert_eq!(
        report
            .errors()
            .map(nebula_schema::ValidationError::code)
            .collect::<Vec<_>>(),
        ["type_mismatch"]
    );
    let data = ValidSchema::any()
        .validate(values)
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(data.into_json(), json!([1, 2]));
}
