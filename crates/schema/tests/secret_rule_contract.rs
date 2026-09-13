//! Secret value rules use data, while context predicates use scrubbed context.

use std::{
    assert_matches,
    error::Error,
    sync::{Arc, Mutex},
};

use nebula_schema::{
    AuthoredValue, Field, ResolvedValues, Schema, SecretValue, ValidSchema, ValidationReport,
    ValuePath, VisibilityMode, field_key,
};
use nebula_validator::{Predicate, Rule, RuleOperands};
use proptest::prelude::*;
use serde_json::{Value, json};

const SECRET: &str = "PRIVATE_AGGREGATE_SENTINEL";
const MARKER: &str = "<redacted>";

fn one_of_rule(values: impl Into<RuleOperands>) -> Rule {
    Rule::one_of(values).expect("bounded equality rule")
}

fn predicate_rule(predicate: Predicate) -> Rule {
    Rule::predicate(predicate).expect("bounded predicate rule")
}

fn all_rule(rules: impl IntoIterator<Item = Rule>) -> Rule {
    Rule::all(rules).expect("bounded conjunction")
}

fn any_rule(rules: impl IntoIterator<Item = Rule>) -> Rule {
    Rule::any(rules).expect("bounded disjunction")
}

fn not_rule(rule: Rule) -> Rule {
    Rule::not(rule).expect("bounded negation")
}

fn custom_rule(expression: &str) -> Rule {
    Rule::custom(expression).expect("bounded custom rule")
}

fn described_rule(rule: Rule, message: impl Into<String>) -> Rule {
    rule.with_message(message).expect("bounded described rule")
}

fn resolve(schema: &ValidSchema, input: AuthoredValue) -> Result<ResolvedValues, ValidationReport> {
    schema.validate(input)?.resolve_data()
}

fn object_schema(rule: Rule) -> ValidSchema {
    Schema::builder()
        .add(
            Field::object(field_key!("auth"))
                .add(Field::secret(field_key!("token")))
                .add(Field::number(field_key!("count")))
                .with_rule(rule),
        )
        .build()
        .unwrap()
}

fn input(token: &str, count: Value) -> AuthoredValue {
    AuthoredValue::from_data(json!({"auth": {"token": token, "count": count}})).unwrap()
}

fn assert_sealed(report: &ValidationReport) {
    assert_payload_hidden(report);
    for error in report.errors() {
        assert!(
            error.params().is_empty(),
            "protected diagnostics must not retain public params"
        );
    }
}

fn assert_payload_hidden(report: &ValidationReport) {
    for rendered in [
        format!("{report:?}"),
        report.to_string(),
        serde_json::to_string(report).unwrap(),
    ] {
        assert!(
            !rendered.contains(SECRET),
            "a public report exposed protected input"
        );
    }
    for error in report.errors() {
        let mut cause: &dyn Error = error;
        loop {
            assert!(!format!("{cause:?} {cause}").contains(SECRET));
            assert!(
                cause
                    .downcast_ref::<nebula_validator::foundation::ValidationError>()
                    .is_none()
            );
            match cause.source() {
                Some(next) => cause = next,
                None => break,
            }
        }
    }
}

#[rstest::rstest]
#[case::object(json!({"token": {"nested": SECRET}}))]
#[case::array(json!({"token": [SECRET]}))]
#[case::secret_key(json!({"token": {SECRET: "value"}}))]
#[case::alias(json!({"old_token": {"nested": SECRET}}))]
#[case::canonical_and_alias(json!({"token": [SECRET], "old_token": {"nested": SECRET}}))]
#[case::wrong_root(json!(SECRET))]
fn malformed_declared_secrets_never_escape_aggregate_error_causes(#[case] wire: Value) {
    let schema = Schema::builder()
        .add(
            Field::secret(field_key!("token"))
                .read_alias("old_token")
                .unwrap(),
        )
        .root_rule(one_of_rule([json!({})]))
        .build()
        .unwrap();
    let report = resolve(&schema, AuthoredValue::from_data(wire).unwrap()).unwrap_err();
    assert!(report.errors().any(|error| error.code() == "type_mismatch"));
    assert!(report.errors().any(|error| error.code() == "one_of"));
    assert_payload_hidden(&report);
}

#[rstest::rstest]
#[case::object(json!({"auth": {"token": {"nested": SECRET}}}))]
#[case::wrong_object(json!({"auth": SECRET}))]
#[case::wrong_list(json!({"tokens": {"nested": SECRET}}))]
#[case::list_item(json!({"tokens": [{"nested": SECRET}]}))]
#[case::unknown_mode(json!({"choice": {"mode": "unknown", "value": {"nested": SECRET}}}))]
fn malformed_secret_containers_seal_field_and_root_rules(#[case] wire: Value) {
    let rejects = one_of_rule([json!(false)]);
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("auth"))
                .add(Field::secret(field_key!("token")))
                .with_rule(rejects.clone()),
        )
        .add(
            Field::list(field_key!("tokens"))
                .item(Field::secret(field_key!("token")))
                .with_rule(rejects.clone()),
        )
        .add(
            Field::mode(field_key!("choice"))
                .variant("token", "Token", Field::secret(field_key!("token")))
                .with_rule(rejects.clone()),
        )
        .root_rule(rejects)
        .build()
        .unwrap();
    let report = resolve(&schema, AuthoredValue::from_data(wire).unwrap()).unwrap_err();
    assert!(report.errors().any(|error| error.code() == "one_of"));
    assert_payload_hidden(&report);
}

#[test]
fn hidden_malformed_secrets_still_fail_without_exposing_their_payload() {
    let wire = json!({"token": {"nested": SECRET}});
    let schema = Schema::builder()
        .add(Field::secret(field_key!("token")).visible(VisibilityMode::Never))
        .root_rule(one_of_rule([json!({})]))
        .build()
        .unwrap();
    let report = resolve(&schema, AuthoredValue::from_data(wire).unwrap()).unwrap_err();
    assert!(report.errors().any(|error| error.code() == "type_mismatch"));
    assert!(report.errors().any(|error| error.code() == "one_of"));
    assert_payload_hidden(&report);
}

#[test]
fn secret_declarations_require_full_rule_access_preflight_even_when_absent() {
    let wire = json!({});
    let schema = Schema::builder()
        .add(Field::secret(field_key!("token")))
        .root_rule(any_rule([
            one_of_rule([wire.clone()]),
            custom_rule("UNSUPPORTED_CALLBACK"),
        ]))
        .build()
        .unwrap();
    let prepared = schema
        .validate(AuthoredValue::from_data(wire).unwrap())
        .unwrap();
    let report = prepared
        .resolve_data()
        .expect_err("aggregate access is governed by the declaration");
    assert_eq!(
        report.errors().next().unwrap().code(),
        "evaluation_unavailable"
    );
    assert_sealed(&report);
}

#[test]
fn root_equality_rejects_redaction_as_secret_data() {
    let schema = Schema::builder()
        .add(Field::secret(field_key!("token")))
        .root_rule(one_of_rule([json!({"token": MARKER})]))
        .build()
        .unwrap();
    let error = resolve(
        &schema,
        AuthoredValue::from_data(json!({"token": SECRET})).unwrap(),
    )
    .expect_err("redaction cannot satisfy equality");
    assert_eq!(error.errors().next().unwrap().code(), "one_of");
    assert_sealed(&error);
}

#[test]
fn root_equality_accepts_actual_secret_data() {
    let schema = Schema::builder()
        .add(Field::secret(field_key!("token")))
        .root_rule(one_of_rule([json!({"token": SECRET})]))
        .build()
        .unwrap();
    let proof = resolve(
        &schema,
        AuthoredValue::from_data(json!({"token": SECRET})).unwrap(),
    )
    .unwrap();
    assert_matches!(
        proof.values().get("token"),
        Some(nebula_schema::ValueTree::Secret(_))
    );
    assert_eq!(proof.values().to_json(), json!({"token": MARKER}));
}

#[rstest::rstest]
#[case::actual(one_of_rule([json!({"token": SECRET, "count": 1})]), true)]
#[case::marker(one_of_rule([json!({"token": MARKER, "count": 1})]), false)]
#[case::not_actual(not_rule(one_of_rule([json!({"token": SECRET, "count": 1})])), false)]
#[case::not_marker(not_rule(one_of_rule([json!({"token": MARKER, "count": 1})])), true)]
#[case::any_actual(any_rule([one_of_rule([json!({"token": SECRET, "count": 1})]), one_of_rule([json!(false)])]), true)]
#[case::any_marker(any_rule([one_of_rule([json!({"token": MARKER, "count": 1})]), one_of_rule([json!(false)])]), false)]
fn object_rules_use_secret_data_under_logic(#[case] rule: Rule, #[case] accepted: bool) {
    let result = resolve(&object_schema(rule), input(SECRET, json!(1)));
    assert_eq!(result.is_ok(), accepted);
    if let Err(report) = result {
        assert_sealed(&report);
    }
}

#[test]
fn literal_redaction_marker_remains_an_ordinary_possible_secret() {
    let schema = object_schema(one_of_rule([json!({"token": MARKER, "count": 1})]));
    let proof = resolve(&schema, input(MARKER, json!(1))).unwrap();
    let Some(nebula_schema::ValueTree::Secret(SecretValue::String(token))) = proof
        .values()
        .get("auth")
        .and_then(|auth| auth.get("token"))
    else {
        panic!("the schema must promote the secret leaf");
    };
    assert_eq!(token.expose(), MARKER);
}

#[rstest::rstest]
#[case::actual(one_of_rule([json!([SECRET, "second"])]), true)]
#[case::marker(one_of_rule([json!([MARKER, MARKER])]), false)]
#[case::negation(not_rule(one_of_rule([json!([SECRET, "second"])])), false)]
#[case::cardinality(all_rule([Rule::min_items(2), Rule::max_items(2)]), true)]
#[case::too_many(Rule::max_items(1), false)]
fn list_rules_validate_real_items(#[case] rule: Rule, #[case] accepted: bool) {
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("tokens"))
                .item(Field::secret(field_key!("token")))
                .with_rule(rule),
        )
        .build()
        .unwrap();
    let result = resolve(
        &schema,
        AuthoredValue::from_data(json!({"tokens": [SECRET, "second"]})).unwrap(),
    );
    assert_eq!(result.is_ok(), accepted);
    if let Err(report) = result {
        assert_sealed(&report);
    }
}

#[rstest::rstest]
#[case::u64_max(json!(u64::MAX), json!(u64::MAX), true)]
#[case::large_distinct(json!(9_007_199_254_740_993_u64), json!(9_007_199_254_740_992_u64), false)]
#[case::float_is_not_integer(json!(1), json!(1.0), false)]
fn aggregate_projection_preserves_json_numeric_identity(
    #[case] actual: Value,
    #[case] expected: Value,
    #[case] accepted: bool,
) {
    let schema = object_schema(one_of_rule([json!({"token": SECRET, "count": expected})]));
    assert_eq!(resolve(&schema, input(SECRET, actual)).is_ok(), accepted);
}

#[rstest::rstest]
#[case::numeric(Rule::min_value(0))]
#[case::described(described_rule(Rule::min_items(2), "submitted {value}"))]
#[case::nested(described_rule(any_rule([Rule::min_value(0), one_of_rule([json!(false)])]), "nested {value}"))]
#[case::literal_message(described_rule(one_of_rule([json!(false)]), SECRET))]
fn every_composite_error_surface_is_sealed(#[case] rule: Rule) {
    let report = resolve(&object_schema(rule), input(SECRET, json!(1))).unwrap_err();
    assert_sealed(&report);
    assert_eq!(report.errors().next().unwrap().path().as_str(), "/auth");
}

#[test]
fn value_rules_and_predicates_use_different_views() {
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("auth"))
                .add(Field::secret(field_key!("token")))
                .add(Field::number(field_key!("count"))),
        )
        .root_rule(all_rule([
            one_of_rule([json!({"auth": {"token": SECRET, "count": 3}})]),
            predicate_rule(Predicate::eq("/auth", json!({"count": 3})).unwrap()),
            predicate_rule(Predicate::eq("/auth/count", json!(3)).unwrap()),
            not_rule(predicate_rule(
                Predicate::eq("/auth", json!({"token": SECRET, "count": 3})).unwrap(),
            )),
        ]))
        .build()
        .unwrap();
    let proof = resolve(&schema, input(SECRET, json!(3))).unwrap();
    assert_eq!(
        proof
            .values()
            .get("auth")
            .unwrap()
            .get("count")
            .unwrap()
            .as_literal(),
        Some(&json!(3))
    );
    let rejected = resolve(&schema, input(SECRET, json!(4))).unwrap_err();
    assert_sealed(&rejected);
}

#[test]
fn non_secret_predicate_diagnostics_keep_their_path_and_message() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("count")))
        .root_rule(described_rule(
            predicate_rule(Predicate::eq("/count", json!(3)).unwrap()),
            "expected {expected}",
        ))
        .build()
        .unwrap();
    let error = resolve(
        &schema,
        AuthoredValue::from_data(json!({"count": 4})).unwrap(),
    )
    .unwrap_err();
    let error = error.errors().next().unwrap();
    assert_eq!(error.code(), "eq_failed");
    assert_eq!(error.path().as_str(), "/count");
    assert!(error.to_string().contains("expected 3"));
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<nebula_validator::foundation::ValidationError>()
            .is_some()
    );
}

#[test]
fn protected_root_predicate_keeps_its_non_secret_error_path() {
    let schema = Schema::builder()
        .add(Field::secret(field_key!("token")))
        .add(Field::number(field_key!("count")))
        .root_rule(predicate_rule(Predicate::eq("/count", json!(3)).unwrap()))
        .build()
        .unwrap();
    let report = resolve(
        &schema,
        AuthoredValue::from_data(json!({"token": SECRET, "count": 4})).unwrap(),
    )
    .unwrap_err();
    assert_eq!(report.errors().next().unwrap().path().as_str(), "/count");
    assert_sealed(&report);
}

#[rstest::rstest]
#[case::actual(SECRET, true)]
#[case::marker(MARKER, false)]
fn select_membership_does_not_compare_redaction(#[case] allowed: &str, #[case] accepted: bool) {
    let schema = Schema::builder()
        .add(Field::select(field_key!("choice")).option(json!({"token": allowed}), "Option"))
        .build()
        .unwrap();
    let mut choice = AuthoredValue::object();
    choice
        .insert(
            "token",
            AuthoredValue::Secret(SecretValue::string(SECRET.into())),
        )
        .unwrap();
    let mut input = AuthoredValue::object();
    input.insert("choice", choice).unwrap();
    let result = resolve(&schema, input);
    assert_eq!(result.is_ok(), accepted);
    if let Err(report) = result {
        assert_sealed(&report);
    }
}

#[rstest::rstest]
#[case::direct(custom_rule("UNSUPPORTED_CALLBACK"))]
#[case::negated(not_rule(custom_rule("UNSUPPORTED_CALLBACK")))]
#[case::alternative(any_rule([one_of_rule([json!({"token": SECRET, "count": 1})]), custom_rule("UNSUPPORTED_CALLBACK")]))]
fn unsupported_execution_cannot_mint_a_secret_proof(#[case] rule: Rule) {
    let schema = object_schema(rule);
    let prepared = schema.validate(input(SECRET, json!(1))).unwrap();
    let report = prepared
        .resolve_data()
        .expect_err("no custom evaluator is admitted to plaintext");
    assert_eq!(
        report.errors().next().unwrap().code(),
        "evaluation_unavailable"
    );
    assert_sealed(&report);
}

fn unique_schema() -> ValidSchema {
    Schema::builder()
        .add(
            Field::list(field_key!("items"))
                .item(Field::object(field_key!("item")))
                .unique(),
        )
        .build()
        .unwrap()
}

fn secret_item(token: &str, number: Value) -> AuthoredValue {
    let mut item = AuthoredValue::from_data(json!({"number": number})).unwrap();
    item.insert(
        "",
        AuthoredValue::Secret(SecretValue::string(token.to_owned())),
    )
    .unwrap();
    item
}

fn unique_input(items: Vec<AuthoredValue>) -> AuthoredValue {
    let mut input = AuthoredValue::object();
    let items = items
        .into_iter()
        .map(|item| {
            let mut wrapped = AuthoredValue::object();
            wrapped.insert("payload", item).unwrap();
            wrapped
        })
        .collect();
    input.insert("items", AuthoredValue::List(items)).unwrap();
    input
}

#[rstest::rstest]
#[case(256)]
#[case(1024)]
#[case(4096)]
fn wide_secret_collections_remain_unique(#[case] count: usize) {
    let items = (0..count)
        .map(|index| secret_item(&format!("token-{index}"), json!(1)))
        .collect();
    let proof = resolve(&unique_schema(), unique_input(items)).unwrap();
    assert_matches!(proof.values().get("items"), Some(nebula_schema::ValueTree::List(items)) if items.len() == count);
}

#[test]
fn unique_items_preserve_numeric_normalization_and_secret_identity() {
    let schema = unique_schema();
    let items = vec![
        secret_item(SECRET, json!(1)),
        secret_item(SECRET, json!(1.0)),
    ];
    let report = resolve(&schema, unique_input(items)).unwrap_err();
    let error = report.errors().next().unwrap();
    assert_eq!(error.code(), "items.unique");
    assert_eq!(error.path().as_str(), "/items/1");
    let items = vec![
        secret_item(SECRET, json!(9_007_199_254_740_993_u64)),
        secret_item(SECRET, json!(9_007_199_254_740_992_u64)),
    ];
    let proof = resolve(&schema, unique_input(items)).unwrap();
    assert_matches!(proof.values().get("items"), Some(nebula_schema::ValueTree::List(items)) if items.len() == 2);
}

#[test]
fn unique_items_distinguish_kinds_markers_and_escaped_paths() {
    let mut left = AuthoredValue::object();
    left.insert(
        "a/b",
        AuthoredValue::Secret(SecretValue::string("ab".into())),
    )
    .unwrap();
    let mut right = AuthoredValue::object();
    right
        .insert(
            "a~1b",
            AuthoredValue::Secret(SecretValue::string("ab".into())),
        )
        .unwrap();
    let items = vec![
        AuthoredValue::Secret(SecretValue::string("ab".into())),
        AuthoredValue::Secret(SecretValue::bytes(b"ab".to_vec())),
        AuthoredValue::from_data(json!(MARKER)).unwrap(),
        left,
        right,
    ];
    let proof = resolve(&unique_schema(), unique_input(items)).unwrap();
    assert_matches!(proof.values().get("items"), Some(nebula_schema::ValueTree::List(items)) if items.len() == 5);
}

proptest! {
    #[test]
    fn uniqueness_matches_first_duplicate_oracle(tokens in prop::collection::vec("[a-z]{0,8}", 0..60)) {
        let duplicate = tokens.iter().enumerate().find_map(|(index, token)| tokens[..index].contains(token).then_some(index));
        let items = tokens.iter().map(|token| secret_item(token, json!(1))).collect();
        match (resolve(&unique_schema(), unique_input(items)), duplicate) {
            (Ok(proof), None) => prop_assert_eq!(proof.values().get("items").unwrap().to_json().as_array().unwrap().len(), tokens.len()),
            (Err(report), Some(index)) => {
                let error = report.errors().next().unwrap();
                prop_assert_eq!(error.code(), "items.unique");
                prop_assert_eq!(error.path(), &ValuePath::root().push("items").push(index.to_string()));
            },
            (result, duplicate) => prop_assert!(false, "unexpected uniqueness result: {result:?}, {duplicate:?}"),
        }
    }
}

#[derive(Clone, Default)]
struct TraceCapture(Arc<Mutex<Vec<String>>>);

impl tracing::field::Visit for TraceCapture {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.lock().unwrap().push(format!("{field}: {value:?}"));
    }
}

impl tracing::Subscriber for TraceCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        attributes.record(&mut self.clone());
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        values.record(&mut self.clone());
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        event.record(&mut self.clone());
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn traces_do_not_publish_composite_rule_input_or_described_errors() {
    let capture = TraceCapture::default();
    tracing::subscriber::with_default(capture.clone(), || {
        let schema = object_schema(described_rule(
            one_of_rule([json!(false)]),
            format!("{SECRET} {{value}}"),
        ));
        assert_sealed(&resolve(&schema, input(SECRET, json!(1))).unwrap_err());
    });
    let records = capture.0.lock().unwrap();
    assert!(
        !records.is_empty(),
        "the trace capture must observe validation"
    );
    assert!(!records.join("\n").contains(SECRET));
}
