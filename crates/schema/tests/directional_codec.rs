use nebula_schema::{
    AuthoredValue, Expression, HasSchema, InputCodec, PropertyType, Schema, SchemaDirection,
    ValuePath, ValueTree, schema_type,
};
use serde_json::json;

#[schema_type(both)]
#[derive(Debug, PartialEq)]
struct Names {
    #[serde(rename(serialize = "emitted", deserialize = "consumed"), alias = "old")]
    value: u32,
}

#[schema_type(both)]
#[derive(Debug, PartialEq)]
struct Maybe(Option<u32>);

#[schema_type(both)]
#[derive(Debug, PartialEq)]
struct Sequence(Vec<u32>);

#[schema_type(both)]
#[derive(Debug, PartialEq)]
struct Recursive {
    value: u32,
    next: Option<Box<Recursive>>,
}

#[schema_type(both)]
#[derive(Debug, PartialEq)]
enum Message {
    Empty,
    Number(u32),
    Record { value: u32 },
}

#[schema_type(both)]
#[derive(Debug, PartialEq)]
enum VariantRenamed {
    #[serde(rename_all = "camelCase")]
    Record { some_value: u32 },
    #[serde(rename_all(serialize = "SCREAMING_SNAKE_CASE", deserialize = "PascalCase"))]
    Split { other_value: u32 },
}

#[schema_type(both)]
#[derive(Debug, PartialEq)]
#[serde(tag = "kind", content = "data")]
enum Adjacent {
    #[serde(alias = "old")]
    Record { value: u32 },
}

#[schema_type(input)]
#[derive(Debug, PartialEq)]
struct Defaults {
    #[property(input(default = "literal"))]
    value: Option<String>,
    #[field(default = 0.1)]
    precise: f32,
}

#[schema_type(input)]
#[derive(Debug, PartialEq)]
struct NullDefault {
    #[property(input(default = null))]
    limit: Option<u32>,
}

#[schema_type(input)]
struct ExpressionField {
    #[field(expression_required)]
    value: u32,
}

#[derive(Schema)]
struct StructuralRecursive {
    next: Option<Box<StructuralRecursive>>,
}

#[derive(nebula_schema::EnumSelect, Schema)]
enum SelectFirst {
    One,
}

#[derive(Schema, nebula_schema::EnumSelect)]
enum SchemaFirst {
    One,
}

#[schema_type(both)]
#[derive(nebula_schema::EnumSelect)]
enum OwnedSelect {
    One,
}

#[schema_type(both)]
#[derive(nebula_schema::EnumSelect)]
#[serde(rename_all = "snake_case")]
enum NativeCatalog {
    One,
}

#[schema_type(both)]
struct NativeCatalogField {
    #[field(enum_select)]
    choice: NativeCatalog,
}

#[schema_type(both)]
struct MismatchedCatalogField {
    #[field(enum_select)]
    choice: OwnedSelect,
}

#[derive(nebula_schema::EnumSelect)]
enum CatalogChoice {
    One,
}

#[derive(Schema)]
struct CatalogField {
    #[field(enum_select)]
    choice: Option<CatalogChoice>,
}

#[schema_type(input)]
struct RecursiveCollections {
    #[property(validate(items(min = 1)))]
    children: Vec<RecursiveCollections>,
    #[field(label = "Nested")]
    nested: Vec<Vec<String>>,
}

#[schema_type(input)]
#[schema(condition(enabled, is_true(field(active))))]
struct NamedInput {
    #[serde(rename = "ready")]
    active: bool,
}

#[test]
fn owned_directional_names_are_executable_and_custody_checked() {
    let input = nebula_schema::InputContract::for_type::<Names>().unwrap();
    let output = nebula_schema::OutputContract::for_type::<Names>().unwrap();
    let value: Names = input
        .validate_data(json!({"old": 7}))
        .unwrap()
        .into_typed(&input)
        .unwrap();
    assert_eq!(value, Names { value: 7 });
    output
        .validate_data(&serde_json::to_value(value).unwrap())
        .unwrap();
    assert!(output.validate_data(&json!({"consumed": 7})).is_err());
    let other = nebula_schema::InputContract::for_type::<u32>().unwrap();
    assert!(
        !input
            .validate_data(json!({"consumed": 7}))
            .unwrap()
            .belongs_to(&other)
    );
    assert_eq!(
        input.record().readmit_input().unwrap().record(),
        input.record()
    );
    assert!(input.record().readmit_output().is_err());
}

#[test]
fn native_nullable_array_enum_and_recursive_roots_validate_exactly() {
    let maybe = nebula_schema::InputContract::for_type::<Maybe>().unwrap();
    assert_eq!(
        maybe
            .validate_data(json!(null))
            .unwrap()
            .into_typed::<Maybe>(&maybe)
            .unwrap(),
        Maybe(None)
    );
    let sequence = nebula_schema::InputContract::for_type::<Sequence>().unwrap();
    assert_eq!(
        sequence
            .validate_data(json!([1, 2]))
            .unwrap()
            .into_typed::<Sequence>(&sequence)
            .unwrap(),
        Sequence(vec![1, 2])
    );
    assert!(sequence.validate_data(json!([1, "bad"])).is_err());
    let recursive = nebula_schema::InputContract::for_type::<Recursive>().unwrap();
    let value: Recursive = recursive
        .validate_data(json!({"value": 1, "next": {"value": 2, "next": null}}))
        .unwrap()
        .into_typed(&recursive)
        .unwrap();
    assert_eq!(value.next.unwrap().value, 2);
    let messages = nebula_schema::OutputContract::for_type::<Message>().unwrap();
    for message in [
        Message::Empty,
        Message::Number(7),
        Message::Record { value: 9 },
    ] {
        messages
            .validate_data(&serde_json::to_value(message).unwrap())
            .unwrap();
    }
    assert!(messages.validate_data(&json!({"Number": "bad"})).is_err());
    assert!(
        nebula_schema::ValidSchema::from_graph(
            &Sequence::definition(SchemaDirection::Input).unwrap()
        )
        .is_err()
    );
}

#[schema_type(both)]
#[derive(Debug, PartialEq)]
struct Port(u16);

#[schema_type(both)]
#[derive(Debug, PartialEq)]
struct Endpoint {
    port: Option<Port>,
}

#[schema_type(input)]
#[derive(Debug, PartialEq)]
struct DefaultedEndpoint {
    #[property(input(default = null))]
    port: Option<Port>,
}

#[test]
fn optional_newtype_admits_null_like_serde() {
    let input = nebula_schema::InputContract::for_type::<Endpoint>().unwrap();
    for (wire, expected) in [
        (json!({"port": null}), None),
        (json!({"port": 8080}), Some(Port(8080))),
    ] {
        let decoded: Endpoint = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(decoded.port, expected);
        let typed: Endpoint = input
            .validate_data(wire)
            .unwrap()
            .into_typed(&input)
            .unwrap();
        assert_eq!(typed.port, expected);
    }
    // The newtype's own domain still applies to non-null values.
    assert!(input.validate_data(json!({"port": 70000})).is_err());
    let output = nebula_schema::OutputContract::for_type::<Endpoint>().unwrap();
    output
        .validate_data(&serde_json::to_value(Endpoint { port: None }).unwrap())
        .unwrap();
    // A null default on the optional newtype is admitted and applied.
    let defaulted = nebula_schema::InputContract::for_type::<DefaultedEndpoint>().unwrap();
    let value: DefaultedEndpoint = defaulted
        .validate_data(json!({}))
        .unwrap()
        .into_typed(&defaulted)
        .unwrap();
    assert_eq!(value.port, None);
}

fn six() -> u32 {
    6
}

fn seven() -> u32 {
    7
}

fn some_text() -> Option<String> {
    Some("text".to_owned())
}

fn one_item() -> Vec<u32> {
    vec![1]
}

#[derive(Schema, serde::Deserialize)]
struct AgreeingDefault {
    #[field(default = 7)]
    #[serde(default = "seven")]
    value: u32,
}

#[derive(Schema, serde::Deserialize)]
struct DisagreeingDefault {
    #[field(default = 7)]
    #[serde(default = "six")]
    value: u32,
}

#[derive(Schema, serde::Deserialize)]
struct DisagreeingNullDefault {
    #[property(input(default = null))]
    #[serde(default = "some_text")]
    value: Option<String>,
}

#[derive(PropertyType, serde::Deserialize)]
struct DisagreeingEmptyDefault {
    #[property(input(default = []))]
    #[serde(default = "one_item")]
    values: Vec<u32>,
}

#[test]
fn serde_default_provider_must_yield_the_literal_schema_default() {
    AgreeingDefault::definition(SchemaDirection::Input).unwrap();
    let decoded: AgreeingDefault = serde_json::from_value(json!({})).unwrap();
    assert_eq!(decoded.value, 7);
    for report in [
        DisagreeingDefault::definition(SchemaDirection::Input).unwrap_err(),
        DisagreeingNullDefault::definition(SchemaDirection::Input).unwrap_err(),
        DisagreeingEmptyDefault::definition(SchemaDirection::Input).unwrap_err(),
    ] {
        assert!(
            report
                .errors()
                .any(|error| error.code() == "schema.codec.default_mismatch"),
            "{report:?}"
        );
    }
    // The outbound graph carries no input default and needs no agreement.
    DisagreeingDefault::definition(SchemaDirection::Output).unwrap();
    let _ = serde_json::from_value::<DisagreeingDefault>(json!({})).map(|value| value.value);
    let _ = serde_json::from_value::<DisagreeingNullDefault>(json!({})).map(|value| value.value);
    let _ = serde_json::from_value::<DisagreeingEmptyDefault>(json!({})).map(|value| value.values);
}

#[test]
fn variant_rename_all_renames_struct_payload_fields_like_serde() {
    let input = nebula_schema::InputContract::for_type::<VariantRenamed>().unwrap();
    let output = nebula_schema::OutputContract::for_type::<VariantRenamed>().unwrap();
    for (value, wire_in) in [
        (
            VariantRenamed::Record { some_value: 7 },
            json!({"Record": {"someValue": 7}}),
        ),
        (
            VariantRenamed::Split { other_value: 9 },
            json!({"Split": {"OtherValue": 9}}),
        ),
    ] {
        // Serde agrees with the schema on the inbound wire...
        let decoded: VariantRenamed = serde_json::from_value(wire_in.clone()).unwrap();
        assert_eq!(decoded, value);
        let typed: VariantRenamed = input
            .validate_data(wire_in)
            .unwrap()
            .into_typed(&input)
            .unwrap();
        assert_eq!(typed, value);
        // ...and on the outbound wire.
        output
            .validate_data(&serde_json::to_value(&value).unwrap())
            .unwrap();
    }
    assert_eq!(
        serde_json::to_value(VariantRenamed::Split { other_value: 9 }).unwrap(),
        json!({"Split": {"OTHER_VALUE": 9}})
    );
    // The unrenamed Rust spelling is not the wire key in either direction.
    assert!(
        input
            .validate_data(json!({"Record": {"some_value": 7}}))
            .is_err()
    );
    assert!(
        output
            .validate_data(&json!({"Record": {"some_value": 7}}))
            .is_err()
    );
    assert!(
        output
            .validate_data(&json!({"Split": {"OtherValue": 9}}))
            .is_err()
    );
}

#[test]
fn owned_literal_defaults_and_adjacent_aliases_are_prepared_exactly() {
    let input = nebula_schema::InputContract::for_type::<Defaults>().unwrap();
    let value: Defaults = input
        .validate_data(json!({}))
        .unwrap()
        .into_typed(&input)
        .unwrap();
    assert_eq!(value.value.as_deref(), Some("literal"));
    assert_eq!(value.precise, 0.1_f32);
    let input = nebula_schema::InputContract::for_type::<Adjacent>().unwrap();
    assert_eq!(
        input
            .validate_data(json!({"kind":"old","data":{"value":7}}))
            .unwrap()
            .into_typed::<Adjacent>(&input)
            .unwrap(),
        Adjacent::Record { value: 7 }
    );
    assert!(
        input
            .validate_data(json!({"kind":"old","data":{"value":"bad"}}))
            .is_err()
    );
    nebula_schema::OutputContract::for_type::<Adjacent>()
        .unwrap()
        .validate_data(&json!({"kind":"Record","data":{"value":7}}))
        .unwrap();
}

#[test]
fn null_allowed_by_option_skips_value_rules_in_admission_and_runtime() {
    // A bounded integer use carries range rules. A null allowed by Option
    // skips non-null value rules in admission (the null default) and at
    // runtime (an omitted or explicit null), per PHASE5_PROPERTY.md.
    let input = nebula_schema::InputContract::for_type::<NullDefault>().unwrap();
    for (data, expected) in [
        (json!({}), None),
        (json!({"limit": null}), None),
        (json!({"limit": 5}), Some(5)),
    ] {
        let value: NullDefault = input
            .validate_data(data)
            .unwrap()
            .into_typed(&input)
            .unwrap();
        assert_eq!(value.limit, expected);
    }
    assert!(input.validate_data(json!({"limit": -1})).is_err());
}

#[test]
fn root_expression_authorization_is_local_and_symbolic_proofs_cannot_resolve() {
    let input = nebula_schema::InputContract::for_type::<ExpressionField>().unwrap();
    let mut values = indexmap::IndexMap::new();
    values.insert(
        "value".to_owned(),
        ValueTree::Expression(Expression::new("{{ 7 }}")),
    );
    let prepared = input.validate(ValueTree::Object(values)).unwrap();
    assert_eq!(prepared.expression_paths(), &[ValuePath::single("value")]);
    assert!(prepared.resolve_data().is_err());
    assert!(
        input
            .validate(ValueTree::Expression(Expression::new("{{ 7 }}")))
            .is_err()
    );
    let scalar = nebula_schema::InputContract::for_type::<u32>().unwrap();
    assert!(
        scalar
            .validate(ValueTree::Expression(Expression::new("{{ 7 }}")))
            .is_err()
    );
    let symbolic = input
        .validate_symbolic(
            AuthoredValue::from_data(json!({})).unwrap(),
            &[ValuePath::single("value")],
        )
        .unwrap();
    assert!(!symbolic.pending().is_empty());
    assert!(symbolic.resolve_data().is_err());
    assert!(
        input
            .validate_symbolic(
                AuthoredValue::from_data(json!({})).unwrap(),
                &[ValuePath::single("missing")]
            )
            .is_err()
    );
}

#[test]
fn standalone_recursive_structure_has_no_legacy_cache_deadlock() {
    StructuralRecursive::definition(SchemaDirection::Input).unwrap();
    assert!(StructuralRecursive::schema().is_err());
    assert!(StructuralRecursive { next: None }.next.is_none());
    assert_eq!(ExpressionField { value: 7 }.value, 7);
    SelectFirst::definition(SchemaDirection::Input).unwrap();
    SchemaFirst::definition(SchemaDirection::Input).unwrap();
    OwnedSelect::input_definition().unwrap();
    let _ = (SelectFirst::One, SchemaFirst::One, OwnedSelect::One);
}

#[test]
fn catalog_only_fields_and_annotated_recursive_collections_keep_exact_graphs() {
    let catalog = CatalogField::definition(SchemaDirection::Input).unwrap();
    let input = nebula_schema::InputContract::from_graph(&catalog).unwrap();
    input.validate_data(json!({"choice":null})).unwrap();
    input.validate_data(json!({"choice":"one"})).unwrap();
    assert!(
        input
            .validate_data(json!({"choice":"outside-catalog"}))
            .is_err()
    );
    assert!(
        CatalogField {
            choice: Some(CatalogChoice::One)
        }
        .choice
        .is_some()
    );
    let input = nebula_schema::InputContract::for_type::<RecursiveCollections>().unwrap();
    assert!(
        input
            .validate_data(json!({"children":[],"nested":[]}))
            .is_err()
    );
    let value = RecursiveCollections {
        children: Vec::new(),
        nested: vec![vec!["text".to_owned()]],
    };
    assert_eq!(value.children.len(), 0);
    assert_eq!(value.nested[0][0], "text");
}

#[test]
fn owned_catalog_facets_preserve_the_actual_child_codec_graph() {
    let input = nebula_schema::InputContract::for_type::<NativeCatalogField>().unwrap();
    let value: NativeCatalogField = input
        .validate_data(json!({"choice":"one"}))
        .unwrap()
        .into_typed(&input)
        .unwrap();
    assert!(matches!(value.choice, NativeCatalog::One));
    let output = nebula_schema::OutputContract::for_type::<NativeCatalogField>().unwrap();
    output
        .validate_data(&serde_json::to_value(value).unwrap())
        .unwrap();
    if let Ok(input) = nebula_schema::InputContract::for_type::<MismatchedCatalogField>() {
        assert!(input.validate_data(json!({"choice":"one"})).is_err());
    }
    if let Ok(output) = nebula_schema::OutputContract::for_type::<MismatchedCatalogField>() {
        assert!(output.validate_data(&json!({"choice":"one"})).is_err());
    }
    let _ = MismatchedCatalogField {
        choice: OwnedSelect::One,
    };
}

#[test]
fn owned_named_condition_reads_canonical_prepared_snapshot() {
    let input = nebula_schema::InputContract::for_type::<NamedInput>().unwrap();
    let condition = input.named_condition("enabled").unwrap();
    let resolved = input.validate_data(json!({"ready":true})).unwrap();
    assert!(condition.matches(resolved.predicate_context()).unwrap());
    let value: NamedInput = resolved.into_typed(&input).unwrap();
    assert!(value.active);
}
