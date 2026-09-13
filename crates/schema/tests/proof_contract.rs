//! Custody boundaries between authoring, pending checks, and usable runtime data.

use nebula_schema::{
    AuthoredValue, EngineExpressionContext, ExpressionMode, Field, Predicate, Rule, Schema,
    SecretValue, Transformer, ValidSchema, ValuePath, ValueTree, field_key, schema_of,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(PartialEq, Zeroize, ZeroizeOnDrop)]
struct TestSecret(String);

impl nebula_schema::SecretInput for TestSecret {}

impl std::fmt::Debug for TestSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TestSecret(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for TestSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

#[test]
fn uniqueness_keeps_numeric_equivalence_inside_secret_bearing_items() {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("items")).unique().item(
                Field::object(field_key!("item"))
                    .add(Field::secret(field_key!("token")))
                    .add(Field::number(field_key!("number"))),
            ),
        )
        .build()
        .unwrap();
    let values = AuthoredValue::from_data(json!({"items": [
        {"token": "same", "number": 1},
        {"token": "same", "number": 1.0}
    ]}))
    .unwrap();

    let error = schema.validate(values).unwrap_err();
    assert_eq!(
        error
            .errors()
            .map(|error| (error.code(), error.path().to_string()))
            .collect::<Vec<_>>(),
        [("items.unique", "/items/1".to_owned())],
    );

    for items in [
        json!([{"token": "a", "number": 1}, {"token": "b", "number": 1.0}]),
        json!([
            {"token": "a", "number": 9_007_199_254_740_992_u64},
            {"token": "a", "number": 9_007_199_254_740_993_u64}
        ]),
    ] {
        let input = AuthoredValue::from_data(json!({"items": items})).unwrap();
        let result = schema.validate(input).unwrap().resolve_data().unwrap();
        assert_eq!(result.into_json()["items"].as_array().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn conditional_requiredness_is_pending_until_its_expression_is_known() {
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("enabled")).expression_mode(ExpressionMode::Allowed))
        .add(
            Field::string(field_key!("token")).required_when(
                Rule::predicate(Predicate::eq("enabled", json!(true)).unwrap())
                    .expect("bounded requiredness rule"),
            ),
        )
        .build()
        .unwrap();
    let input = AuthoredValue::from_template_json(json!({
        "enabled": {"$expr": "{{ $input.enabled }}"}
    }))
    .unwrap();
    let valid = schema.validate(input).unwrap();
    assert!(
        valid
            .pending()
            .iter()
            .any(|pending| pending.path().to_string() == "/token")
    );

    let error = valid
        .clone()
        .resolve(&EngineExpressionContext::with_input(
            json!({"enabled": true}),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error
            .errors()
            .map(|error| (error.code(), error.path().to_string()))
            .collect::<Vec<_>>(),
        [("required", "/token".to_owned())],
    );
    let resolved = valid
        .resolve(&EngineExpressionContext::with_input(
            json!({"enabled": false}),
        ))
        .await
        .unwrap();
    assert_eq!(resolved.into_json(), json!({"enabled": false}));
}

#[tokio::test]
async fn a_predicate_on_an_expression_is_not_a_predicate_on_missing_data() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("tier")))
        .root_rule(
            Rule::predicate(Predicate::Ne(
                ValuePath::from_pointer("/tier").unwrap(),
                json!("blocked"),
            ))
            .expect("bounded root predicate"),
        )
        .build()
        .unwrap();
    let input = AuthoredValue::from_template_json(json!({"tier": "{{ $input.tier }}"})).unwrap();
    let valid = schema.validate(input).unwrap();
    assert!(!valid.pending().is_empty());
    let error = valid
        .resolve(&EngineExpressionContext::with_input(
            json!({"tier": "blocked"}),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error
            .errors()
            .map(nebula_schema::ValidationError::code)
            .collect::<Vec<_>>(),
        ["ne_failed"]
    );
}

#[test]
fn data_only_completion_rejects_admitted_programs() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("value")))
        .build()
        .unwrap();
    let input = AuthoredValue::from_template_json(json!({"value": "{{ $input.value }}"})).unwrap();
    let error = schema.validate(input).unwrap().resolve_data().unwrap_err();
    assert_eq!(
        error
            .errors()
            .map(|error| (error.code(), error.path().to_string()))
            .collect::<Vec<_>>(),
        [("expression.forbidden", "/value".to_owned())],
    );
}

#[test]
fn data_ingress_preserves_template_syntax_and_expression_shaped_objects() {
    let input = json!({"literal": "{{ $input.token }}", "object": {"$expr": "{{ 1 / 0 }}"}});
    let schema = ValidSchema::any();
    let prepared = schema
        .validate(AuthoredValue::from_data(input.clone()).unwrap())
        .unwrap();
    assert_eq!(prepared.pending(), []);
    assert_eq!(prepared.resolve_data().unwrap().into_json(), input);
}

#[test]
fn full_completion_cannot_claim_an_unsupported_rule_was_satisfied() {
    let schema = Schema::builder()
        .root_rule(Rule::custom("private_runtime_check").expect("bounded custom rule"))
        .build()
        .unwrap();
    let prepared = schema.validate(AuthoredValue::object()).unwrap();
    assert!(!prepared.pending().is_empty());
    let error = prepared.resolve_data().unwrap_err();
    assert_eq!(error.errors().count(), 1);
    assert!(!format!("{error:?}").contains("private_runtime_check"));
}

#[derive(Debug, Deserialize, Schema, PartialEq)]
enum Authentication {
    Token {
        #[field(secret)]
        #[serde(alias = "legacy_token")]
        token: TestSecret,
    },
}

#[test]
fn explicit_secret_decoding_uses_the_schema_union_and_alias_contract() {
    let schema = schema_of::<Authentication>().unwrap();
    let input = schema
        .values_from_wire(json!({"Token": {"legacy_token": "private-token"}}))
        .unwrap();
    assert_eq!(schema.project(&input).unwrap(), json!({"Token": {}}));
    let prepared = schema.validate(input).unwrap();
    assert_eq!(prepared.to_wire_json(), json!({"Token": {}}));
    let resolved = prepared.resolve_data().unwrap();
    assert_eq!(resolved.to_wire_json(), json!({"Token": {}}));
    assert_eq!(
        resolved
            .clone()
            .into_typed_exposing_secrets::<Authentication>()
            .unwrap(),
        Authentication::Token {
            token: TestSecret("private-token".to_owned())
        },
    );
    let error = resolved.into_typed::<Authentication>().unwrap_err();
    assert!(!format!("{error:?} {error}").contains("private-token"));
}

#[test]
fn data_only_secret_normalization_is_retained_exactly_once() {
    let schema = Schema::builder()
        .add(
            Field::secret(field_key!("token"))
                .read_alias("legacy_token")
                .unwrap()
                .with_transformer(Transformer::Replace {
                    from: "a".into(),
                    to: "aa".into(),
                }),
        )
        .build()
        .unwrap();
    let input = AuthoredValue::from_data(json!({"legacy_token": "a"})).unwrap();
    let resolved = schema.validate(input).unwrap().resolve_data().unwrap();
    let SecretValue::String(secret) = resolved.get_secret(&field_key!("token")).unwrap() else {
        panic!("expected a protected string");
    };
    assert_eq!(secret.expose(), "aa");
    assert_eq!(
        resolved.into_typed_exposing_secrets::<Value>().unwrap(),
        json!({"token": "aa"})
    );
}

#[test]
fn arbitrary_data_keys_and_roots_do_not_become_schema_identifiers() {
    for input in [
        json!(null),
        json!(5),
        json!([1, 2]),
        json!({"": {"/~": {"0": "data"}}}),
    ] {
        let resolved = ValidSchema::any()
            .validate(AuthoredValue::from_data(input.clone()).unwrap())
            .unwrap()
            .resolve_data()
            .unwrap();
        assert_eq!(resolved.into_json(), input);
    }
    let data = AuthoredValue::from_data(json!({"": {"/~": {"0": "data"}}})).unwrap();
    let pointer = ValuePath::from_pointer("//~1~0/0").unwrap();
    assert_eq!(
        data.get_path(&pointer).and_then(ValueTree::as_str),
        Some("data")
    );
}
