//! Root shape follows serde data, independently of declaration field count.

use std::{assert_matches, fmt::Debug};

use nebula_schema::{
    AuthoredValue, HasSchema, PathResolveError, PathWalk, RootShape, Rule, ScalarKind,
    ScalarSchema, Schema, SchemaKind, ValidSchema, ValuePath, schema_of,
};
use nebula_validator::ValueRule;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Number, Value, json};

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema)]
struct UnitStruct;

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "braces preserve serde's object wire shape, distinct from a unit struct's null"
)]
struct EmptyBraces {}

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema)]
enum Mixed {
    Unit,
    Empty {},
}

fn roundtrip<T: HasSchema + Serialize + DeserializeOwned + Debug + PartialEq>(value: T) {
    let wire = serde_json::to_value(&value).unwrap();
    let schema = schema_of::<T>().unwrap();
    let resolved = schema
        .validate(schema.values_from_wire(wire.clone()).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(resolved.to_wire_json(), wire);
    assert_eq!(resolved.into_typed::<T>().unwrap(), value);
}

#[test]
fn unit_and_empty_record_follow_distinct_serde_shapes() {
    assert_eq!(serde_json::to_value(()).unwrap(), Value::Null);
    assert_eq!(serde_json::to_value(UnitStruct).unwrap(), Value::Null);
    assert_eq!(serde_json::to_value(EmptyBraces {}).unwrap(), json!({}));
    let unit = schema_of::<()>().unwrap();
    let empty = schema_of::<EmptyBraces>().unwrap();
    assert_eq!(unit, schema_of::<UnitStruct>().unwrap());
    assert_eq!(unit.scalar_schema().unwrap().kind(), ScalarKind::Null);
    assert_eq!(empty.kind(), SchemaKind::Record);
    assert_ne!(unit, empty);
    assert_ne!(unit, ValidSchema::any());
    assert_ne!(empty, ValidSchema::any());
    roundtrip(());
    roundtrip(UnitStruct);
    roundtrip(EmptyBraces {});
    roundtrip(Mixed::Unit);
    roundtrip(Mixed::Empty {});
}

#[test]
fn known_primitives_preserve_values_and_exact_extremes() {
    roundtrip(true);
    roundtrip("{{ $data.secret }}".to_owned());
    roundtrip(i8::MIN);
    roundtrip(i8::MAX);
    roundtrip(i64::MIN);
    roundtrip(i64::MAX);
    roundtrip(u64::MAX);
    roundtrip(f32::MIN);
    roundtrip(f32::MAX);
    roundtrip(f64::MIN);
    roundtrip(f64::MAX);
    roundtrip(f64::MIN_POSITIVE);
    roundtrip(f64::from_bits(1));
}

#[test]
fn integer_domain_rejects_wrong_shape_and_out_of_range_before_decode() {
    let schema = schema_of::<i8>().unwrap();
    for rejected in [
        json!(-129),
        json!(128),
        json!(1.5),
        json!("1"),
        json!(null),
        json!({}),
        json!([]),
    ] {
        let report = schema
            .validate(AuthoredValue::from_data(rejected).unwrap())
            .unwrap_err();
        assert!(report.has_errors());
        assert_eq!(report.errors().next().unwrap().path(), &ValuePath::root());
    }
    let unsigned = schema_of::<u64>().unwrap();
    for rejected in [json!(-1), json!(18_446_744_073_709_551_616.0)] {
        assert!(
            unsigned
                .validate(AuthoredValue::from_data(rejected).unwrap())
                .is_err()
        );
    }
}

#[test]
fn integral_float_is_losslessly_normalized_and_retained() {
    let schema = schema_of::<i8>().unwrap();
    let prepared = schema
        .validate(AuthoredValue::from_data(json!(1.0)).unwrap())
        .unwrap();
    assert_eq!(prepared.values().to_json(), json!(1));
    assert_eq!(
        prepared.resolve_data().unwrap().into_typed::<i8>().unwrap(),
        1
    );
    let zero = schema
        .validate(AuthoredValue::from_data(json!(-0.0)).unwrap())
        .unwrap();
    assert_eq!(zero.values().to_json(), json!(0));
}

#[test]
fn scalar_roots_forbid_programs_but_strings_are_only_data() {
    let schema = schema_of::<String>().unwrap();
    let expression = AuthoredValue::from_template_json(json!("{{ $data.value }}")).unwrap();
    let report = schema.validate(expression).unwrap_err();
    assert_eq!(
        report.errors().next().unwrap().code(),
        "expression.forbidden"
    );
    roundtrip("{{ $data.value }}".to_owned());
}

#[test]
fn scalar_rules_are_not_lost_when_the_root_has_no_fields() {
    let schema = ValidSchema::scalar(
        ScalarSchema::string()
            .root_rule(Rule::value(ValueRule::MinLength(3)).expect("bounded scalar root rule")),
    )
    .unwrap();
    assert!(schema.fields().is_empty());
    assert_eq!(schema.root_rules().len(), 1);
    let report = schema
        .validate(AuthoredValue::from_data(json!("ab")).unwrap())
        .unwrap_err();
    assert_eq!(report.errors().next().unwrap().code(), "min_length");
    assert_eq!(report.errors().next().unwrap().path(), &ValuePath::root());
    let valid = schema
        .validate(AuthoredValue::from_data(json!("abc")).unwrap())
        .unwrap();
    assert_eq!(
        valid
            .resolve_data()
            .unwrap()
            .into_typed::<String>()
            .unwrap(),
        "abc"
    );
}

#[test]
fn scalar_wire_is_versioned_without_changing_historical_roots() {
    assert_eq!(
        serde_json::to_string(&ValidSchema::empty()).unwrap(),
        r#"{"fields":[]}"#
    );
    assert_eq!(
        serde_json::to_string(&ValidSchema::any()).unwrap(),
        r#"{"kind":"any","fields":[]}"#
    );
    let schema = schema_of::<i8>().unwrap();
    let wire =
        r#"{"kind":"scalar","scalar":{"version":1,"type":"integer","minimum":-128,"maximum":127}}"#;
    assert_eq!(serde_json::to_string(&schema).unwrap(), wire);
    assert_eq!(serde_json::from_str::<ValidSchema>(wire).unwrap(), schema);
    assert_eq!(
        serde_json::from_str::<ValidSchema>(r#"{"fields":[]}"#).unwrap(),
        ValidSchema::empty()
    );
    assert_eq!(nebula_schema::SCHEMA_WIRE_VERSION, 1);
}

#[test]
fn scalar_serde_rejects_invalid_or_contradictory_contracts() {
    for wire in [
        json!({"kind":"scalar"}),
        json!({"kind":"scalar","scalar":{"version":2,"type":"null"}}),
        json!({"kind":"scalar","scalar":{"version":1,"type":"integer","minimum":2,"maximum":1}}),
        json!({"kind":"scalar","scalar":{"version":1,"type":"integer","minimum":0.5,"maximum":2}}),
        json!({"kind":"scalar","scalar":{"version":1,"type":"number"}}),
        json!({"kind":"scalar","scalar":{"version":1,"type":"null","minimum":null}}),
        json!({"kind":"scalar","fields":[],"scalar":{"version":1,"type":"null"}}),
        json!({"kind":"scalar","scalar":{"version":1,"type":"null","unknown":true}}),
        json!({"fields":[],"scalar":{"version":1,"type":"null"}}),
    ] {
        assert!(
            serde_json::from_value::<ValidSchema>(wire.clone()).is_err(),
            "accepted {wire}"
        );
    }
    assert!(ScalarSchema::integer(2, 1).is_err());
    assert!(ScalarSchema::integer(Number::from_f64(0.5).unwrap(), 2).is_err());
    assert!(ScalarSchema::number(2, 1).is_err());
}

#[derive(Serialize, Deserialize, Schema)]
struct NestedUnit {
    inner: UnitStruct,
}

#[derive(Serialize, Deserialize, Schema)]
struct UnitList {
    items: Vec<UnitStruct>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema)]
struct NestedUnknown {
    inner: Value,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Schema)]
struct UnknownList {
    items: Vec<Value>,
}

#[derive(Serialize, Deserialize, Schema)]
struct NestedEmpty {
    inner: EmptyBraces,
}

#[test]
fn nested_scalar_roots_are_rejected_instead_of_erased() {
    for report in [
        schema_of::<NestedUnit>().unwrap_err(),
        schema_of::<UnitList>().unwrap_err(),
    ] {
        assert_eq!(
            report.errors().next().unwrap().code(),
            "derive.unsupported_nested_root"
        );
    }
    let schema = schema_of::<NestedEmpty>().unwrap();
    let resolved = schema
        .validate(AuthoredValue::from_data(json!({"inner":{}})).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(resolved.to_wire_json(), json!({"inner":{}}));
}

#[test]
fn derived_any_fields_are_dynamic_and_preserve_opaque_data() {
    let schema = schema_of::<NestedUnknown>().unwrap();
    assert_matches!(schema.fields()[0], nebula_schema::Field::Dynamic(_));
    assert_eq!(
        schema.walk_reference_path(&ValuePath::from_pointer("/inner/arbitrary/path").unwrap()),
        PathWalk::Opaque
    );
    for value in [
        json!(false),
        json!(9),
        json!("{{ $data.value }}"),
        json!([]),
        json!({"": {"a/b~c": [null, true]}}),
    ] {
        roundtrip(NestedUnknown { inner: value });
    }
    let list_schema = schema_of::<UnknownList>().unwrap();
    let nebula_schema::Field::List(list) = &list_schema.fields()[0] else {
        panic!("expected list");
    };
    assert_matches!(list.item.as_deref(), Some(nebula_schema::Field::Dynamic(_)));
    roundtrip(UnknownList {
        items: vec![
            Value::Null,
            json!(false),
            json!(2),
            json!("text"),
            json!({}),
            json!([]),
        ],
    });
}

#[test]
fn derived_any_fields_do_not_admit_expressions_in_opaque_descendants() {
    let schema = schema_of::<NestedUnknown>().unwrap();
    let values =
        AuthoredValue::from_template_json(json!({"inner":{"child":"{{ $data.value }}"}})).unwrap();
    let report = schema.validate(values).unwrap_err();
    let error = report.errors().next().unwrap();
    assert_eq!(error.code(), "expression.forbidden");
    assert_eq!(error.path().to_string(), "/inner/child");
}

#[derive(Serialize, Deserialize, Schema)]
#[schema(custom = "payload_guard")]
struct GuardedPayload {
    label: String,
}

#[derive(Serialize, Deserialize, Schema)]
enum GuardedExternal {
    Payload(GuardedPayload),
}

#[derive(Serialize, Deserialize, Schema)]
#[serde(tag = "kind", content = "value")]
enum GuardedAdjacent {
    Payload(GuardedPayload),
}

#[test]
fn union_newtype_payloads_cannot_silently_drop_record_root_rules() {
    assert_eq!(schema_of::<GuardedPayload>().unwrap().root_rules().len(), 1);
    for report in [
        schema_of::<GuardedExternal>().unwrap_err(),
        schema_of::<GuardedAdjacent>().unwrap_err(),
    ] {
        assert_eq!(
            report.errors().next().unwrap().code(),
            "union.newtype_root_rules"
        );
    }
    assert_eq!(
        serde_json::to_value(schema_of::<GuardedExternal>().unwrap_err()).unwrap(),
        serde_json::to_value(schema_of::<GuardedExternal>().unwrap_err()).unwrap(),
    );
}

#[test]
fn scalar_paths_cannot_descend_into_properties() {
    let schema = schema_of::<bool>().unwrap();
    assert_matches!(schema.root_shape(), RootShape::Scalar(_));
    assert_eq!(
        schema.walk_reference_path(&ValuePath::root()),
        PathWalk::ResolvedRoot
    );
    assert_matches!(
        schema.walk_reference_path(&ValuePath::from_pointer("/child").unwrap()),
        PathWalk::Unresolved(PathResolveError::DescendPastLeaf { .. })
    );
}
