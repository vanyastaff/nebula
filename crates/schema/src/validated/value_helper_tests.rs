use serde_json::{Value, json};

use super::{SerdeTagging, ValidSchema};
use crate::{AuthoredValue, Expression, Property, ResolvedValue, Schema, ValueTree, field_key};

fn assert_data_only(value: &AuthoredValue) {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            ValueTree::Literal(_) => {},
            ValueTree::Object(properties) => pending.extend(properties.values()),
            ValueTree::List(items) => pending.extend(items),
            ValueTree::Expression(_) | ValueTree::Secret(_) => {
                panic!("wire data must not acquire expression or secret syntax")
            },
        }
    }
}

fn union(tagging: SerdeTagging) -> ValidSchema {
    ValidSchema::union(
        Property::mode(field_key!("auth"))
            .variant("data", "Data", Property::object(field_key!("payload")))
            .variant_empty("none", "None"),
        tagging,
    )
    .unwrap()
}

#[test]
fn wire_ingress_preserves_arbitrary_data_without_authored_interpretation() {
    let data = json!({
        "": "{{ $data.source }}",
        "a/b~": {"$expr": "$data.source"},
        "\u{e9}": [{"kind": "expression", "value": "$data.source"}],
    });
    for schema in [ValidSchema::empty(), ValidSchema::any()] {
        let values = schema.values_from_wire(data.clone()).unwrap();
        assert_data_only(&values);
        assert_eq!(values.to_json(), data);
        assert_eq!(schema.raw_values_to_wire(values.to_json()), data);
    }
    for data in [
        Value::Null,
        json!(7),
        json!("{{ $data.x }}"),
        json!([1, {"": 2}]),
    ] {
        let values = ValidSchema::any().values_from_wire(data.clone()).unwrap();
        assert_data_only(&values);
        assert_eq!(values.to_json(), data);
    }
}

#[test]
fn union_data_and_unit_wire_roundtrip_for_both_taggings() {
    let payload = json!({"": "{{ $data.x }}", "$expr": "$data.y", "a/b~": [1, 2]});
    for (schema, data_wire, unit_wire) in [
        (
            union(SerdeTagging::External),
            json!({"data": payload}),
            json!("none"),
        ),
        (
            union(SerdeTagging::Adjacent {
                tag: "type".into(),
                content: "body".into(),
            }),
            json!({"type": "data", "body": payload}),
            json!({"type": "none"}),
        ),
    ] {
        for wire in [data_wire, unit_wire] {
            let values = schema.values_from_wire(wire.clone()).unwrap();
            assert_data_only(&values);
            assert_eq!(schema.raw_values_to_wire(values.to_json()), wire);
        }
    }
}

#[test]
fn malformed_union_reverse_mapping_does_not_discard_data() {
    let schema = union(SerdeTagging::External);
    for data in [
        json!({"auth": {"mode": "missing", "value": {"": 1}}}),
        json!({"auth": {"mode": "none", "value": null}}),
        json!({"auth": {"mode": "data"}}),
        json!({"auth": {"mode": "data", "value": {}, "extra": true}}),
        json!({"auth": {"mode": {"unexpected": 1}}}),
    ] {
        assert_eq!(schema.raw_values_to_wire(data.clone()), data);
    }
}

#[test]
fn undeclared_paths_preserve_empty_and_escaped_keys_across_stages() {
    let schema = Schema::builder()
        .property(Property::object(field_key!("nested")))
        .build()
        .unwrap();
    for key in ["", "/", "~", "a/b~", "\u{e9}"] {
        let data = json!({"nested": {key: 1}});
        let expected = crate::ValuePath::root().push("nested").push(key);
        let authored = AuthoredValue::from_data(data.clone()).unwrap();
        let resolved = ResolvedValue::from_data(data).unwrap();
        assert_eq!(
            schema.first_undeclared_path(&authored),
            Some(expected.clone())
        );
        assert_eq!(schema.first_undeclared_path(&resolved), Some(expected));
    }
    let empty_key = AuthoredValue::from_data(json!({"": null})).unwrap();
    assert_eq!(
        ValidSchema::empty()
            .first_undeclared_path(&empty_key)
            .unwrap()
            .as_str(),
        "/"
    );
}

#[test]
fn undeclared_paths_walk_list_payloads_and_reject_extra_mode_keys() {
    let schema = Schema::builder()
        .property(Property::list(field_key!("rows")).item(Property::object(field_key!("row"))))
        .build()
        .unwrap();
    let values = AuthoredValue::from_data(json!({"rows": [{"a/b~": true}]})).unwrap();
    assert_eq!(
        schema.first_undeclared_path(&values).unwrap().as_str(),
        "/rows/0/a~1b~0"
    );

    let schema = union(SerdeTagging::External);
    let values = schema
        .values_from_wire(json!({"data": {"": true}}))
        .unwrap();
    assert_eq!(
        schema.first_undeclared_path(&values).unwrap().as_str(),
        "/auth/value/"
    );
    let values = AuthoredValue::from_data(json!({"auth": {"mode": "none", "extra": 1}})).unwrap();
    assert_eq!(
        schema.first_undeclared_path(&values).unwrap().as_str(),
        "/auth/extra"
    );
}

#[test]
fn projection_folds_aliases_and_omits_secrets_without_mutating_input() {
    let schema = Schema::builder()
        .property(
            Property::object(field_key!("settings"))
                .read_alias("legacy_settings")
                .unwrap()
                .property(
                    Property::string(field_key!("id"))
                        .read_alias("legacy_id")
                        .unwrap()
                        .emit_as("public_id")
                        .unwrap(),
                )
                .property(
                    Property::secret(field_key!("token"))
                        .read_alias("legacy_token")
                        .unwrap(),
                ),
        )
        .build()
        .unwrap();
    let values = AuthoredValue::from_data(json!({"legacy_settings": {
        "id": "canonical", "legacy_id": "losing",
        "token": "protected", "legacy_token": "also protected",
        "public_id": "spoofed", "": true, "a/b~": 3
    }}))
    .unwrap();
    let before = values.clone();
    assert_eq!(
        schema.project(&values).unwrap(),
        json!({"settings": {
            "public_id": "canonical", "": true, "a/b~": 3
        }})
    );
    assert_eq!(values, before);
}

#[test]
fn projection_reserves_absent_outputs_and_preserves_expression_envelopes() {
    let schema = Schema::builder()
        .property(
            Property::string(field_key!("id"))
                .emit_as("public_id")
                .unwrap(),
        )
        .build()
        .unwrap();
    let values = AuthoredValue::from_data(json!({"public_id": "spoofed", "a/b~": 1})).unwrap();
    assert_eq!(schema.project(&values).unwrap(), json!({"a/b~": 1}));

    let mut values = AuthoredValue::object();
    values
        .insert("id", AuthoredValue::Expression(Expression::new("$data.id")))
        .unwrap();
    assert_eq!(
        schema.project(&values).unwrap(),
        json!({"public_id": {"$expr": "$data.id"}})
    );
}

#[test]
fn malformed_mode_selectors_do_not_leak_payload_or_select_defaults() {
    let schema = Schema::builder()
        .property(
            Property::mode(field_key!("auth"))
                .variant(
                    "known",
                    "Known",
                    Property::object(field_key!("payload"))
                        .property(Property::string(field_key!("id")))
                        .property(Property::secret(field_key!("token"))),
                )
                .default_variant("known"),
        )
        .build()
        .unwrap();
    let values = AuthoredValue::from_data(json!({"auth": {
        "mode": {"token": "must not escape"}, "value": {"id": "not selected", "token": "protected"}
    }}))
    .unwrap();
    assert_eq!(schema.project(&values).unwrap(), json!({"auth": {}}));
    let values = AuthoredValue::from_data(
        json!({"auth": {"value": {"id": "selected", "token": "protected"}}}),
    )
    .unwrap();
    assert_eq!(
        schema.project(&values).unwrap(),
        json!({"auth": {"value": {"id": "selected"}}})
    );
}

#[test]
fn public_projection_checks_raw_depth_before_the_recursive_walk() {
    let schema = ValidSchema::any();
    let mut data = Value::Null;
    for _ in 0..crate::value::MAX_VALUE_DEPTH {
        data = json!([data]);
    }
    let values = AuthoredValue::from_data(data.clone()).unwrap();
    assert_eq!(schema.project(&values).unwrap(), data);
    let over_depth = AuthoredValue::List(vec![values]);
    assert_eq!(
        schema.project(&over_depth).unwrap_err().code(),
        "recursion_limit"
    );
    let scalar = AuthoredValue::from_data(json!(1)).unwrap();
    assert_eq!(
        ValidSchema::empty().project(&scalar).unwrap_err().code(),
        "type_mismatch"
    );
}
