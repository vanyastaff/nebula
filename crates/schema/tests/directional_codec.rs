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
