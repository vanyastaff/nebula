use super::super::model::{Body, EmptyPolicy, NullPolicy, PresencePolicy, ValueProtection};
use super::*;
use crate::{ExpressionMode, FieldAliases, ScalarSchema, Schema, VisibilityMode, field_key};

fn root_body(graph: &AdmittedSchemaGraph) -> &Body {
    let index = graph.0.lookup[&graph.0.graph.root.0.target];
    &graph.0.graph.definitions[index.0].body
}

#[test]
fn every_legacy_scalar_retains_its_known_domain_and_bounds() {
    let cases = [
        ScalarSchema::null(),
        ScalarSchema::boolean(),
        ScalarSchema::string(),
        ScalarSchema::integer(-17, 29).unwrap(),
        ScalarSchema::number(
            serde_json::Number::from_f64(-1.5).unwrap(),
            serde_json::Number::from_f64(8.25).unwrap(),
        )
        .unwrap(),
    ];
    for scalar in cases {
        let graph = ValidSchema::scalar(scalar.clone())
            .unwrap()
            .to_admitted_graph()
            .unwrap();
        match (scalar.kind(), root_body(&graph)) {
            (ScalarKind::Null, Body::Null)
            | (ScalarKind::Boolean, Body::Boolean { .. })
            | (ScalarKind::String, Body::String { .. }) => {},
            (ScalarKind::Integer, Body::Integer(numeric))
            | (ScalarKind::Number, Body::Number(numeric)) => {
                assert_eq!(numeric.minimum.as_ref(), scalar.minimum());
                assert_eq!(numeric.maximum.as_ref(), scalar.maximum());
            },
            _ => panic!("known scalar domain was changed"),
        }
    }
    assert!(matches!(
        root_body(&ValidSchema::any().to_admitted_graph().unwrap()),
        Body::Any
    ));
    assert!(matches!(
        root_body(&ValidSchema::empty().to_admitted_graph().unwrap()),
        Body::Record { .. }
    ));
}

#[test]
fn required_policies_secrets_expressions_rules_and_aliases_survive() {
    let aliases = FieldAliases::new(["old_name"]).unwrap();
    let schema = Schema::builder()
        .property(
            Property::string(field_key!("name"))
                .required()
                .min_length(3)
                .read_aliases(aliases)
                .emit_as("new_name")
                .unwrap(),
        )
        .property(Property::secret(field_key!("token")).required())
        .property(Property::boolean(field_key!("flag")))
        .build()
        .unwrap();
    let graph = schema.to_admitted_graph().unwrap();
    let Body::Record { properties, .. } = root_body(&graph) else {
        panic!("record required")
    };
    let name = properties
        .iter()
        .find(|property| property.key.as_str() == "name")
        .unwrap();
    assert!(matches!(name.presence, PresencePolicy::Required));
    assert!(matches!(name.core.null, NullPolicy::Reject));
    assert!(matches!(name.core.empty_string, EmptyPolicy::Reject));
    assert_eq!(name.core.expression, ExpressionMode::Allowed);
    assert_eq!(name.core.rules, schema.properties()[0].rules());
    assert_eq!(name.aliases.read[0].as_str(), "old_name");
    assert_eq!(name.aliases.write.as_ref().unwrap().as_str(), "new_name");
    let token = properties
        .iter()
        .find(|property| property.key.as_str() == "token")
        .unwrap();
    let flag = properties
        .iter()
        .find(|property| property.key.as_str() == "flag")
        .unwrap();
    assert_eq!(token.core.protection, ValueProtection::SecretUtf8);
    assert!(matches!(flag.presence, PresencePolicy::Optional));
    assert!(matches!(flag.core.null, NullPolicy::Reject));
}

#[test]
fn nested_records_and_lists_retain_element_shape_and_constraints() {
    let schema = Schema::builder()
        .property(
            Property::list(field_key!("items"))
                .required()
                .min_items(2)
                .max_items(9)
                .unique()
                .item(
                    Property::object(field_key!("item"))
                        .property(Property::string(field_key!("name"))),
                ),
        )
        .build()
        .unwrap();
    let graph = schema.to_admitted_graph().unwrap();
    let Body::Record { properties, .. } = root_body(&graph) else {
        panic!("record required")
    };
    assert!(matches!(
        properties[0].core.empty_collection,
        EmptyPolicy::Reject
    ));
    let array_index = graph.0.lookup[&properties[0].core.target];
    let Body::Array(array) = &graph.0.graph.definitions[array_index.0].body else {
        panic!("array required")
    };
    assert_eq!(array.min_items, 2);
    assert_eq!(array.max_items, Some(9));
    assert!(array.unique);
    let item_index = graph.0.lookup[&array.element.0.target];
    let Body::Record { properties, .. } = &graph.0.graph.definitions[item_index.0].body else {
        panic!("object element required")
    };
    assert_eq!(properties[0].key.as_str(), "name");
}

#[test]
fn presentation_and_default_hints_never_change_semantic_identity_or_fill_inputs() {
    let make = |label: &str, default: Value| {
        Schema::builder()
            .property(
                Property::string(field_key!("name"))
                    .required()
                    .label(label)
                    .default(default)
                    .visible(VisibilityMode::Never),
            )
            .build()
            .unwrap()
            .to_admitted_graph()
            .unwrap()
    };
    let first = make("First", json!("a"));
    let second = make("Second", json!("b"));
    assert_eq!(first.semantic_commitment(), second.semantic_commitment());
    let Body::Record { properties, .. } = root_body(&first) else {
        panic!("record required")
    };
    assert!(properties[0].input_default.is_none());
    assert!(matches!(properties[0].presence, PresencePolicy::Required));
}

#[test]
fn unsupported_legacy_domains_and_conditions_fail_closed() {
    for property in [
        Property::integer(field_key!("integer")).into_property(),
        Property::string(field_key!("conditional"))
            .required_when(
                nebula_validator::Rule::predicate(
                    nebula_validator::Predicate::eq("flag", true).unwrap(),
                )
                .unwrap(),
            )
            .into_property(),
        Property::file(field_key!("file")).into_property(),
        Property::select(field_key!("select")).into_property(),
        Property::computed(field_key!("computed")).into_property(),
        Property::dynamic(field_key!("dynamic"))
            .loader("dynamic_loader")
            .into_property(),
        Property::notice(field_key!("notice")).into_property(),
    ] {
        assert!(
            Schema::builder()
                .property(Property::boolean(field_key!("flag")))
                .property(property)
                .build()
                .unwrap()
                .to_admitted_graph()
                .is_err()
        );
    }
    let historical: ValidSchema = serde_json::from_value(json!({"fields": []})).unwrap();
    assert!(historical.to_admitted_graph().is_err());
}

#[test]
fn graph_native_root_arrays_are_never_flattened_to_any_by_the_legacy_boundary() {
    let graph = serde_json::from_value::<SchemaGraphDocument>(json!({
        "version": 3, "root": {"target": "items", "null": "reject"},
        "definitions": [
            {"key": "items", "body": {"kind": "array", "element": {"target": "text", "null": "reject"}}},
            {"key": "text", "body": {"kind": "string"}}
        ]
    })).unwrap().admit().unwrap();
    assert!(ValidSchema::from_graph(&graph).is_err());
    assert!(matches!(root_body(&graph), Body::Array(_)));
}

#[test]
fn nullable_graph_roots_are_not_widened_to_legacy_any() {
    let graph = serde_json::from_value::<SchemaGraphDocument>(json!({
        "version": 3, "root": {"target": "text", "null": "allow"},
        "definitions": [{"key": "text", "body": {"kind": "string"}}]
    }))
    .unwrap()
    .admit()
    .unwrap();
    assert!(ValidSchema::from_graph(&graph).is_err());
    assert!(matches!(root_body(&graph), Body::String { .. }));
}

#[test]
fn executable_rules_transformers_and_expression_policies_affect_identity() {
    let make = |field: crate::StringField| {
        Schema::builder()
            .property(field)
            .build()
            .unwrap()
            .to_admitted_graph()
            .unwrap()
    };
    let plain = make(Property::string(field_key!("text")));
    let bounded = make(Property::string(field_key!("text")).min_length(2));
    let forbidden = make(Property::string(field_key!("text")).no_expression());
    let mut transformed = Property::string(field_key!("text"));
    transformed.transformers.push(crate::Transformer::Trim);
    let transformed = make(transformed);
    assert_ne!(plain.semantic_commitment(), bounded.semantic_commitment());
    assert_ne!(plain.semantic_commitment(), forbidden.semantic_commitment());
    assert_ne!(
        plain.semantic_commitment(),
        transformed.semantic_commitment()
    );
    let Body::Record { properties, .. } = root_body(&transformed) else {
        panic!("record required")
    };
    assert_eq!(properties[0].core.transformers, [crate::Transformer::Trim]);
}
