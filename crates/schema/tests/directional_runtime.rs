use nebula_schema::{
    AdmittedSchemaGraph, AuthoredValue, EvalFuture, Expression, ExpressionContext, InputCodec,
    InputContract, OutputContract, PropertyType, SchemaGraphDocument, SchemaTypeBuilder,
    SchemaTypeUse, SecretValue, ValidationReport, ValuePath, ValueTree, schema_type,
};
use serde_json::{Value, json};

fn graph(root: Value, definitions: Value) -> AdmittedSchemaGraph {
    serde_json::from_value::<SchemaGraphDocument>(
        json!({"version":3,"root":root,"definitions":definitions}),
    )
    .unwrap()
    .admit()
    .unwrap()
}

fn expression(source: &str) -> AuthoredValue {
    ValueTree::Expression(Expression::new(source))
}

struct Results {
    right: u32,
}
impl ExpressionContext for Results {
    fn evaluate<'a>(&'a self, program: &'a nebula_schema::CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move {
            Ok(json!(if program.source().contains("left") {
                1
            } else {
                self.right
            }))
        })
    }
}

#[test]
fn alias_null_and_closed_domain_intersections_are_enforced() {
    let aliases = graph(
        json!({"target":"alias","null":"allow"}),
        json!([
            {"key":"alias","body":{"kind":"alias","alias":{"target":"text","null":"reject"}}},
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    // The occurrence that admits null decides it (serde's `Option<Newtype>`);
    // the alias's wrapped occurrence still governs non-null values.
    let aliases = InputContract::from_graph(&aliases).unwrap();
    assert_eq!(
        aliases
            .validate_data(Value::Null)
            .unwrap()
            .into_wire_data()
            .unwrap(),
        Value::Null
    );
    assert!(aliases.validate_data(json!(7)).is_err());
    let closed = graph(
        json!({"target":"text","null":"allow","accepted_domain":{"closed":["allowed"]}}),
        json!([
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    assert!(
        InputContract::from_graph(&closed)
            .unwrap()
            .validate_data(Value::Null)
            .is_err()
    );
    let nullable = graph(
        json!({"target":"text","null":"allow","accepted_domain":{"closed":[null,"allowed"]}}),
        json!([
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    assert_eq!(
        InputContract::from_graph(&nullable)
            .unwrap()
            .validate_data(Value::Null)
            .unwrap()
            .into_wire_data()
            .unwrap(),
        Value::Null
    );
    let defaulted = graph(
        json!({"target":"record","null":"reject"}),
        json!([
            {"key":"record","body":{"kind":"record","properties":[{"key":"value","target":"text","presence":"optional","null":"allow","input_default":null}],"additional_properties":"closed"}},
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    assert_eq!(
        InputContract::from_graph(&defaulted)
            .unwrap()
            .validate_data(json!({}))
            .unwrap()
            .into_wire_data()
            .unwrap(),
        json!({"value":null})
    );
    // A null default admitted by its own occurrence does not consult the
    // alias's wrapped occurrence, as at runtime (`Option<Newtype>` defaults).
    let inner_rejects: SchemaGraphDocument = serde_json::from_value(json!({"version":3,"root":{"target":"record","null":"reject"},"definitions":[
        {"key":"record","body":{"kind":"record","properties":[{"key":"value","target":"alias","presence":"optional","null":"allow","input_default":null}],"additional_properties":"closed"}},
        {"key":"alias","body":{"kind":"alias","alias":{"target":"text","null":"reject"}}},
        {"key":"text","body":{"kind":"string"}}
    ]})).unwrap();
    assert_eq!(
        InputContract::from_graph(&inner_rejects.admit().unwrap())
            .unwrap()
            .validate_data(json!({}))
            .unwrap()
            .into_wire_data()
            .unwrap(),
        json!({"value":null})
    );
    // A null default on an occurrence that rejects null is still refused.
    let outer_rejects: SchemaGraphDocument = serde_json::from_value(json!({"version":3,"root":{"target":"record","null":"reject"},"definitions":[
        {"key":"record","body":{"kind":"record","properties":[{"key":"value","target":"alias","presence":"optional","null":"reject","input_default":null}],"additional_properties":"closed"}},
        {"key":"alias","body":{"kind":"alias","alias":{"target":"text","null":"allow"}}},
        {"key":"text","body":{"kind":"string"}}
    ]})).unwrap();
    assert!(outer_rejects.admit().is_err());
}

#[test]
fn adjacent_union_default_selector_fills_an_absent_tag() {
    let union = |tagging: Value| {
        serde_json::from_value::<SchemaGraphDocument>(json!({"version":3,
        "root":{"target":"choice","null":"reject"},
        "definitions":[
            {"key":"choice","body":{"kind":"union","tagging":tagging,"variants":[
                {"key":"none","payload":null},
                {"key":"some","payload":{"target":"text","null":"reject"}}
            ],"selector_normalization":{"default_variant":"none"}}},
            {"key":"text","body":{"kind":"string"}}
        ]}))
        .unwrap()
        .admit()
    };
    let adjacent = InputContract::from_graph(
        &union(json!({"adjacent":{"tag":"kind","content":"data"}})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        adjacent
            .validate_data(json!({}))
            .unwrap()
            .into_wire_data()
            .unwrap(),
        json!({"kind":"none"})
    );
    // An explicit selector is never replaced by the default.
    assert_eq!(
        adjacent
            .validate_data(json!({"kind":"some","data":"x"}))
            .unwrap()
            .into_wire_data()
            .unwrap(),
        json!({"kind":"some","data":"x"})
    );
    // External tagging has no absent-selector form: the facet is refused.
    assert!(union(json!("external")).is_err());
}

#[test]
fn unique_arrays_compare_whole_values_in_linear_time() {
    let unique = |element: &str, kind: &str, nullability: &str| {
        InputContract::from_graph(&graph(
            json!({"target":"array","null":"reject"}),
            json!([
                {"key":"array","body":{"kind":"array","unique":true,"element":{"target":element,"null":nullability}}},
                {"key":element,"body":{"kind":kind}}
            ]),
        ))
        .unwrap()
    };
    let numbers = unique("number", "number", "reject");
    // Close to the node budget: a pairwise scan needs ~2e9 comparisons here.
    let distinct: Vec<Value> = (0..60_000).map(|index| json!(index)).collect();
    let started = std::time::Instant::now();
    numbers.validate_data(Value::Array(distinct)).unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "uniqueness took {:?}",
        started.elapsed()
    );
    let mut repeated: Vec<Value> = (0..60_000).map(|index| json!(index)).collect();
    repeated.push(json!(59_999));
    assert!(numbers.validate_data(Value::Array(repeated)).is_err());
    // Equality is whole-value JSON equality: key order is irrelevant and an
    // integer differs from its float spelling.
    let any = unique("any", "any", "allow");
    assert!(
        any.validate_data(json!([{"a":1,"b":[2]}, {"b":[2],"a":1}]))
            .is_err()
    );
    any.validate_data(json!([1, 1.0, null, "1", [1], {"1":1}]))
        .unwrap();
}

#[tokio::test]
async fn whole_value_uniqueness_remains_pending_until_all_elements_resolve() {
    let array = graph(
        json!({"target":"array","null":"reject"}),
        json!([
            {"key":"array","body":{"kind":"array","unique":true,"element":{"target":"number","null":"reject","expression":"allowed"}}},
            {"key":"number","body":{"kind":"integer"}}
        ]),
    );
    let input = InputContract::from_graph(&array).unwrap();
    let authored = || ValueTree::List(vec![expression("{{ $left }}"), expression("{{ $right }}")]);
    let prepared = input.validate(authored()).unwrap();
    assert!(prepared.has_pending_root_checks());
    assert_eq!(
        prepared
            .resolve(&Results { right: 2 })
            .await
            .unwrap()
            .into_wire_data()
            .unwrap(),
        json!([1, 2])
    );
    assert!(
        input
            .validate(authored())
            .unwrap()
            .resolve(&Results { right: 1 })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn whole_record_closed_domain_waits_for_its_expression_values() {
    let record = graph(
        json!({"target":"record","null":"reject","accepted_domain":{"closed":[{"x":1}]}}),
        json!([
            {"key":"record","body":{"kind":"record","properties":[{"key":"x","target":"number","null":"reject","expression":"allowed"}],"additional_properties":"closed"}},
            {"key":"number","body":{"kind":"integer"}}
        ]),
    );
    let input = InputContract::from_graph(&record).unwrap();
    let authored = || {
        let mut values = indexmap::IndexMap::new();
        values.insert("x".to_owned(), expression("{{ $right }}"));
        ValueTree::Object(values)
    };
    let prepared = input.validate(authored()).unwrap();
    assert!(prepared.has_pending_root_checks());
    assert_eq!(
        prepared
            .resolve(&Results { right: 1 })
            .await
            .unwrap()
            .into_wire_data()
            .unwrap(),
        json!({"x":1})
    );
    assert!(
        input
            .validate(authored())
            .unwrap()
            .resolve(&Results { right: 2 })
            .await
            .is_err()
    );
}

#[schema_type(both)]
struct First(u32);
#[schema_type(both)]
struct Second(u32);

#[schema_type(input)]
struct TypedWrapperInput {
    #[field(expression_required)]
    value: First,
}

#[test]
fn alias_edges_compose_expression_restrictions_before_program_admission() {
    let contract = |mode: &str| {
        InputContract::from_graph(&graph(
        json!({"target":"alias","null":"reject","expression":"allowed"}),
        json!([
            {"key":"alias","body":{"kind":"alias","alias":{"target":"number","null":"reject","expression":mode}}},
            {"key":"number","body":{"kind":"integer"}}
        ])
    )).unwrap()
    };
    assert!(
        contract("forbidden")
            .validate(expression("{{ 7 }}"))
            .is_err()
    );
    assert!(contract("required").validate_data(json!(7)).is_err());
    contract("required")
        .validate(expression("{{ 7 }}"))
        .unwrap();
    contract("allowed").validate(expression("{{ 7 }}")).unwrap();
    let input = InputContract::for_type::<TypedWrapperInput>().unwrap();
    let mut authored = indexmap::IndexMap::new();
    authored.insert("value".to_owned(), expression("{{ 7 }}"));
    assert_eq!(
        input
            .validate(ValueTree::Object(authored))
            .unwrap()
            .expression_paths(),
        &[ValuePath::single("value")]
    );
    assert_eq!(TypedWrapperInput { value: First(7) }.value.0, 7);
}

#[test]
fn same_graph_cannot_upgrade_untyped_or_different_type_custody() {
    let first = InputContract::for_type::<First>().unwrap();
    let second = InputContract::for_type::<Second>().unwrap();
    assert_eq!(first.graph(), second.graph());
    assert!(!first.validate_data(json!(7)).unwrap().belongs_to(&second));
    assert!(
        first
            .validate_data(json!(7))
            .unwrap()
            .into_typed::<Second>(&second)
            .is_err()
    );
    let untyped = InputContract::from_graph(first.graph()).unwrap();
    assert!(
        untyped
            .validate_data(json!(7))
            .unwrap()
            .into_typed::<First>(&first)
            .is_err()
    );
    let decoded: First = first
        .validate_data(json!(7))
        .unwrap()
        .into_typed(&first)
        .unwrap();
    assert_eq!(decoded.0, 7);
    assert_eq!(Second(8).0, 8);
}

#[test]
fn inactive_and_empty_nested_protected_output_domains_reject_at_admission() {
    let recursive = graph(
        json!({"target":"record","null":"reject"}),
        json!([
            {"key":"record","body":{"kind":"record","properties":[
                {"key":"next","target":"record","presence":"optional","null":"allow"},
                {"key":"password","target":"text","presence":"optional","null":"reject","protection":"secret_utf8"}
            ],"additional_properties":"closed"}},
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    assert!(OutputContract::from_graph(&recursive).is_err());
    let input = InputContract::from_graph(&recursive).unwrap();
    let protected = input
        .graph()
        .input_reference_at(&ValuePath::single("password"))
        .unwrap();
    assert!(!protected.data_populates_protected(&Value::Null));
    assert!(protected.data_populates_protected(&json!("canary-private")));
}

#[test]
fn productive_long_aliases_do_not_multiply_the_value_call_stack() {
    let mut definitions = Vec::new();
    for index in 0..300 {
        definitions.push(json!({"key":format!("alias{index}"),"body":{"kind":"alias","alias":{"target":if index==299 {"record".to_owned()} else {format!("alias{}", index+1)},"null":"allow"}}}));
    }
    definitions.push(json!({"key":"record","body":{"kind":"record","properties":[{"key":"next","target":"alias0","presence":"optional","null":"allow"}],"additional_properties":"closed"}}));
    let input = InputContract::from_graph(&graph(
        json!({"target":"alias0","null":"reject"}),
        json!(definitions),
    ))
    .unwrap();
    let mut value = json!({});
    for _ in 0..16 {
        value = json!({"next":value});
    }
    assert_eq!(
        input
            .validate_data(value.clone())
            .unwrap()
            .into_wire_data()
            .unwrap(),
        value
    );
}

#[test]
fn symbolic_destinations_use_inbound_aliases_and_canonical_pending_paths() {
    let schema = graph(
        json!({"target":"record","null":"allow"}),
        json!([
            {"key":"record","body":{"kind":"record","properties":[{"key":"current","target":"text","presence":"required","null":"reject","aliases":{"read":["old"]}}],"additional_properties":"closed"}},
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    let input = InputContract::from_graph(&schema).unwrap();
    let authored = || AuthoredValue::from_data(json!({})).unwrap();
    let pending = input
        .validate_symbolic(authored(), &[ValuePath::single("old")])
        .unwrap();
    assert!(
        pending
            .pending()
            .iter()
            .any(|pending| pending.path() == &ValuePath::single("current"))
    );
    assert!(pending.resolve_data().is_err());
    assert!(
        input
            .validate_symbolic(
                authored(),
                &[ValuePath::single("old"), ValuePath::single("current")]
            )
            .is_err()
    );
    assert!(
        input
            .validate_symbolic(
                AuthoredValue::from_data(json!({"old":"literal"})).unwrap(),
                &[ValuePath::single("current")]
            )
            .is_err()
    );
}

#[test]
fn local_condition_commitments_preserve_alpha_renaming_and_bind_behavior() {
    let definition = |root: &str, child: &str, expected: bool| {
        let condition = nebula_validator::Condition::try_from(
            nebula_validator::Rule::predicate(
                nebula_validator::Predicate::eq("/active", json!(expected)).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let mut local = serde_json::Map::new();
        local.insert(child.to_owned(), json!({"enabled":condition}));
        serde_json::from_value::<SchemaGraphDocument>(json!({"version":3,
            "root":{"target":root,"null":"reject"},
            "definitions":[
                {"key":root,"body":{"kind":"record","properties":[{"key":"child","target":child,"null":"reject"}],"additional_properties":"closed"}},
                {"key":child,"body":{"kind":"record","properties":[{"key":"active","target":"bool","null":"reject"}],"additional_properties":"closed"}},
                {"key":"bool","body":{"kind":"boolean"}}
            ],"x-nebula-local-conditions":local
        })).unwrap().admit().unwrap()
    };
    let first = InputContract::from_graph(&definition("outer", "inner", true)).unwrap();
    let renamed = InputContract::from_graph(&definition("a", "z", true)).unwrap();
    let changed = InputContract::from_graph(&definition("outer", "inner", false)).unwrap();
    assert_eq!(first.semantic_commitment(), renamed.semantic_commitment());
    assert_ne!(first.semantic_commitment(), changed.semantic_commitment());
    assert!(OutputContract::from_graph(first.graph()).is_err());
    assert_eq!(
        first.record().readmit_input().unwrap().record(),
        first.record()
    );
}

#[derive(serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct ProtectedBytes(String);
impl nebula_schema::SecretInput for ProtectedBytes {}
impl PropertyType for ProtectedBytes {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder
            .define::<Self>(|_| Ok(json!({"kind":"bytes","encoding":"base64"})))
            .map(SchemaTypeUse::secret_bytes)
    }
}
impl InputCodec for ProtectedBytes {}

#[derive(serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct TextSecret(String);
impl nebula_schema::SecretInput for TextSecret {}
impl PropertyType for TextSecret {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        String::define_schema_type(builder).map(SchemaTypeUse::secret_utf8)
    }
}
impl InputCodec for TextSecret {}

#[derive(serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct NumericSecret(u32);
impl nebula_schema::SecretInput for NumericSecret {}
impl PropertyType for NumericSecret {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        u32::define_schema_type(builder)
    }
}
impl InputCodec for NumericSecret {}

#[schema_type(input)]
struct IncompatibleSecretField {
    #[field(secret)]
    secret: NumericSecret,
}

#[schema_type(input)]
struct ProtectedText {
    #[field(secret)]
    password: TextSecret,
}

#[test]
fn protected_byte_and_text_domains_have_explicit_native_wire_and_disclosure() {
    assert!(InputContract::for_type::<IncompatibleSecretField>().is_err());
    let value = IncompatibleSecretField {
        secret: NumericSecret(7),
    };
    assert_eq!(value.secret.0, 7);
    let input = InputContract::for_type::<ProtectedBytes>().unwrap();
    let value: ProtectedBytes = input
        .validate_data(json!("AQID/w=="))
        .unwrap()
        .into_typed_exposing_secrets(&input)
        .unwrap();
    assert_eq!(value.0, "AQID/w==");
    assert!(
        input
            .validate_data(json!("AQID/w=="))
            .unwrap()
            .into_wire_data()
            .is_err()
    );
    assert!(input.validate_data(json!("not base64!")).is_err());
    let wrong = input
        .validate(ValueTree::Secret(SecretValue::string(
            "byte-canary".to_owned(),
        )))
        .unwrap_err();
    assert!(!format!("{wrong:?} {wrong}").contains("byte-canary"));
    let text = InputContract::for_type::<ProtectedText>().unwrap();
    let value: ProtectedText = text
        .validate_data(json!({"password":"text-canary"}))
        .unwrap()
        .into_typed_exposing_secrets(&text)
        .unwrap();
    assert_eq!(value.password.0, "text-canary");
    let mut authored = indexmap::IndexMap::new();
    authored.insert(
        "password".to_owned(),
        ValueTree::Secret(SecretValue::bytes(vec![1, 2, 3])),
    );
    assert!(text.validate(ValueTree::Object(authored)).is_err());
    let alias = graph(
        json!({"target":"alias","null":"reject","protection":"secret_utf8"}),
        json!([
            {"key":"alias","body":{"kind":"alias","alias":{"target":"text","null":"reject","expression":"allowed"}}},
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    let value = InputContract::from_graph(&alias)
        .unwrap()
        .validate_data(json!("alias-canary"))
        .unwrap();
    assert!(matches!(
        value.values(),
        ValueTree::Secret(SecretValue::String(_))
    ));
    assert!(!format!("{value:?}").contains("alias-canary"));
    assert!(value.into_wire_data().is_err());
}
