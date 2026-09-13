//! Authored wire identity, depth, and expression-table contracts.

use std::assert_matches;

use nebula_expression::{CompiledProgram, EvaluationContext, ExpressionEngine};
use nebula_schema::{
    AuthoredValue, CompiledValue, Expression, ProgramSyntax, ResolvedValue, ScalarValue,
    SecretValue, ValuePath,
};
use proptest::prelude::*;
use rstest::rstest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn nested_data(mut leaf: Value, object: bool, depth: usize) -> Value {
    for _ in 0..depth {
        leaf = if object {
            json!({"child": leaf})
        } else {
            json!([leaf])
        };
    }
    leaf
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    authored: AuthoredValue,
}

#[rstest]
#[case::scalar(json!(7))]
#[case::empty_object(json!({}))]
#[case::empty_list(json!([]))]
fn maximum_depth_round_trips_through_json_bytes(
    #[case] leaf: Value,
    #[values(false, true)] object: bool,
) {
    let authored = AuthoredValue::from_data(nested_data(leaf, object, 64)).unwrap();
    let bytes = serde_json::to_vec(&authored).unwrap();
    let decoded: AuthoredValue = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, authored);

    let record = Record { authored };
    let bytes = serde_json::to_vec(&record).unwrap();
    let decoded: Record = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, record);
}

fn envelope(data: Value, expressions: Value) -> Value {
    json!({"version": 2, "data": data, "expressions": expressions})
}

fn decode(wire: &Value) -> Result<AuthoredValue, serde_json::Error> {
    serde_json::from_slice(&serde_json::to_vec(wire).unwrap())
}

fn expression(source: &str) -> AuthoredValue {
    AuthoredValue::Expression(Expression::new(source))
}

#[test]
fn compiled_template_round_trip_preserves_string_result() {
    let compiled = CompiledValue::Expression(CompiledProgram::compile_template("{{ 7 }}").unwrap());
    let bytes = serde_json::to_vec(&compiled).unwrap();
    let authored: AuthoredValue = serde_json::from_slice(&bytes).unwrap();
    let AuthoredValue::Expression(expression) = authored else {
        panic!("the expression slot must survive authored decoding");
    };
    assert_eq!(
        ExpressionEngine::new()
            .evaluate_compiled(expression.parse().unwrap(), &EvaluationContext::new())
            .unwrap(),
        json!("7")
    );
}

#[test]
fn envelope_preserves_literal_and_expression_identity() {
    let literal = json!({
        "template": "{{ secret() }}",
        "explicit": {"$expr": "secret()"},
        "codec": {"version": 2, "data": null, "expressions": [{"path": "", "source": "1"}]},
        "old_tag": {"kind": "expression", "value": "1"},
        "actual": null,
        "list": [null, {"$expr": "2"}, "{{ 3 }}"],
    });
    let mut authored = AuthoredValue::from_data(literal.clone()).unwrap();
    authored
        .insert("actual", expression("{{ 4 + 5 }}"))
        .unwrap();
    let wire = serde_json::to_value(&authored).unwrap();
    assert_eq!(
        wire,
        envelope(
            literal,
            json!([{"path": "/actual", "syntax": "auto", "source": "{{ 4 + 5 }}"}])
        )
    );
    assert_eq!(decode(&wire).unwrap(), authored);
}

#[rstest]
#[case::null(json!(null))]
#[case::boolean(json!(true))]
#[case::signed(json!(i64::MIN))]
#[case::unsigned(json!(u64::MAX))]
#[case::fraction(json!(0.125))]
#[case::rounded_fraction(json!(-9.616_495_922_012_473e-23_f64))]
#[case::maximum_float(json!(f64::MAX))]
#[case::minimum_normal_float(json!(f64::MIN_POSITIVE))]
#[case::string(json!("{{ literal }}"))]
#[case::object(json!({"": {"$expr": "literal"}}))]
#[case::list(json!([null, false, "{{ literal }}"]))]
#[case::empty_object(json!({}))]
#[case::empty_list(json!([]))]
fn arbitrary_root_data_round_trips(#[case] data: Value) {
    let authored = AuthoredValue::from_data(data.clone()).unwrap();
    let wire = serde_json::to_value(&authored).unwrap();
    assert_eq!(wire, envelope(data, json!([])));
    assert_eq!(decode(&wire).unwrap(), authored);
}

#[rstest]
#[case("")]
#[case("{{ 1 + 2 }}")]
#[case("{{ syntax is not checked here")]
fn root_expression_preserves_source_without_parsing(#[case] source: &str) {
    let authored = expression(source);
    let wire = envelope(
        json!(null),
        json!([{"path": "", "syntax": "auto", "source": source}]),
    );
    assert_eq!(serde_json::to_value(&authored).unwrap(), wire);
    assert_eq!(decode(&wire).unwrap(), authored);
    assert_ne!(
        decode(&wire).unwrap(),
        AuthoredValue::from_data(json!(null)).unwrap()
    );
}

#[rstest]
#[case::auto(ProgramSyntax::Auto, "auto")]
#[case::expression(ProgramSyntax::Expression, "expression")]
#[case::template(ProgramSyntax::Template, "template")]
fn syntax_is_explicit_even_when_source_is_malformed(
    #[case] syntax: ProgramSyntax,
    #[case] tag: &str,
) {
    let source = "{{ syntax-sentinel-5ec9 +";
    let authored = AuthoredValue::Expression(Expression::with_syntax(source, syntax));
    let wire = envelope(
        json!(null),
        json!([{"path": "", "syntax": tag, "source": source}]),
    );
    assert_eq!(serde_json::to_value(&authored).unwrap(), wire);
    let decoded = decode(&wire).unwrap();
    assert_eq!(decoded, authored);
    let AuthoredValue::Expression(expression) = decoded else {
        panic!("the authored wire must retain its expression slot");
    };
    assert_eq!(expression.syntax(), syntax);
    let error = expression.parse().unwrap_err();
    assert_eq!(error.code(), "expression.parse");
    assert!(!format!("{expression:?} {error:?} {error}").contains("syntax-sentinel-5ec9"));
}

#[rstest]
#[case::unknown(json!("syntax-sentinel-5ec9"))]
#[case::wrong_case(json!("Template"))]
#[case::null(json!(null))]
#[case::number(json!(123_456_789))]
#[case::boolean(json!(true))]
#[case::object(json!({"syntax-sentinel-5ec9": "template"}))]
#[case::list(json!(["syntax-sentinel-5ec9"]))]
fn invalid_syntax_is_rejected_without_echoing_payload(#[case] syntax: Value) {
    let wire = envelope(
        json!(null),
        json!([{"path": "", "syntax": syntax, "source": "source-sentinel-abc"}]),
    );
    for error in [
        decode(&wire).unwrap_err(),
        serde_json::from_value::<AuthoredValue>(wire).unwrap_err(),
    ] {
        let diagnostic = format!("{error:?} {error}");
        assert!(diagnostic.contains("invalid program syntax"));
        assert!(!diagnostic.contains("syntax-sentinel-5ec9"));
        assert!(!diagnostic.contains("123456789"));
        assert!(!diagnostic.contains("source-sentinel-abc"));
    }
}

#[test]
fn compiled_syntax_round_trips_at_root_list_and_arbitrary_key() {
    let source = "'{{ 7 }}'";
    let mut compiled = CompiledValue::object();
    let mut expected = AuthoredValue::object();
    for (key, syntax, tag) in [
        ("", ProgramSyntax::Auto, "auto"),
        ("/~", ProgramSyntax::Expression, "expression"),
        ("0", ProgramSyntax::Template, "template"),
    ] {
        let program = CompiledProgram::compile_with_syntax(source, syntax).unwrap();
        let root = CompiledValue::Expression(program.clone());
        let root_wire = serde_json::to_value(&root).unwrap();
        assert_eq!(root_wire["expressions"][0]["syntax"], tag);
        assert_eq!(
            decode(&root_wire).unwrap(),
            AuthoredValue::Expression(Expression::with_syntax(source, syntax))
        );
        compiled
            .insert(
                key,
                CompiledValue::List(vec![CompiledValue::Expression(program)]),
            )
            .unwrap();
        expected
            .insert(
                key,
                AuthoredValue::List(vec![AuthoredValue::Expression(Expression::with_syntax(
                    source, syntax,
                ))]),
            )
            .unwrap();
    }
    let wire = serde_json::to_value(&compiled).unwrap();
    assert_eq!(decode(&wire).unwrap(), expected);
    assert_eq!(serde_json::to_value(expected).unwrap(), wire);
}

#[test]
fn property_names_are_escaped_once_and_do_not_use_array_index_rules() {
    let keys = [
        "", "0", "00", "-", "+1", "/", "~", "~1", "a/b", "a~b", "\0", "\u{96ea}",
    ];
    let mut authored = AuthoredValue::object();
    for key in keys {
        authored.insert(key, expression(key)).unwrap();
    }
    let wire = serde_json::to_value(&authored).unwrap();
    let paths: Vec<_> = wire["expressions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "/",
            "/0",
            "/00",
            "/-",
            "/+1",
            "/~1",
            "/~0",
            "/~01",
            "/a~1b",
            "/a~0b",
            "/\0",
            "/\u{96ea}"
        ]
    );
    assert_eq!(decode(&wire).unwrap(), authored);
}

#[test]
fn independent_paths_can_share_encoded_prefixes_in_any_order() {
    let data = json!({"": {"": null}, "a": [null, null], "ab": null, "a/b": null, "a~b": null});
    let mut entries = vec![
        json!({"path": "//", "syntax": "auto", "source": "empty"}),
        json!({"path": "/a/0", "syntax": "auto", "source": "first"}),
        json!({"path": "/a/1", "syntax": "auto", "source": "second"}),
        json!({"path": "/ab", "syntax": "auto", "source": "prefix"}),
        json!({"path": "/a~1b", "syntax": "auto", "source": "slash"}),
        json!({"path": "/a~0b", "syntax": "auto", "source": "tilde"}),
    ];
    let forward = decode(&envelope(data.clone(), json!(entries))).unwrap();
    for entry in &entries {
        let path = ValuePath::from_pointer(entry["path"].as_str().unwrap()).unwrap();
        assert_eq!(
            forward.get_path(&path).unwrap(),
            &expression(entry["source"].as_str().unwrap())
        );
    }
    entries.reverse();
    assert_eq!(decode(&envelope(data, json!(entries))).unwrap(), forward);
}

#[rstest]
#[case::plain_null(json!(null))]
#[case::plain_string(json!("{{ 1 }}"))]
#[case::plain_object(json!({"$expr": "1"}))]
#[case::old_tag(json!({"kind": "literal", "value": null}))]
#[case::empty(json!({}))]
#[case::missing_version(json!({"data": null, "expressions": []}))]
#[case::missing_data(json!({"version": 2, "expressions": []}))]
#[case::missing_expressions(json!({"version": 2, "data": null}))]
#[case::old_version(json!({"version": 1, "data": null, "expressions": []}))]
#[case::future_version(json!({"version": 3, "data": null, "expressions": []}))]
#[case::string_version(json!({"version": "2", "data": null, "expressions": []}))]
#[case::fraction_version(json!({"version": 2.0, "data": null, "expressions": []}))]
#[case::negative_version(json!({"version": -1, "data": null, "expressions": []}))]
#[case::extra_field(json!({"version": 2, "data": null, "expressions": [], "secret": true}))]
#[case::sequence_envelope(json!([2, null, []]))]
#[case::null_table(envelope(json!(null), json!(null)))]
#[case::map_table(envelope(json!(null), json!({"": "1"})))]
#[case::sequence_entry(envelope(json!(null), json!([["", "auto", "1"]])))]
#[case::missing_path(envelope(json!(null), json!([{"syntax": "auto", "source": "1"}])))]
#[case::missing_source(envelope(json!(null), json!([{"path": "", "syntax": "auto"}])))]
#[case::missing_syntax(envelope(json!(null), json!([{"path": "", "source": "1"}])))]
#[case::extra_entry_field(envelope(json!(null), json!([{"path": "", "syntax": "auto", "source": "1", "secret": true}])))]
#[case::non_string_source(envelope(json!(null), json!([{"path": "", "syntax": "auto", "source": null}])))]
#[case::non_string_path(envelope(json!(null), json!([{"path": [], "syntax": "auto", "source": "1"}])))]
fn malformed_envelopes_are_rejected(#[case] wire: Value) {
    assert!(decode(&wire).is_err(), "accepted malformed wire: {wire}");
    assert!(
        serde_json::from_value::<AuthoredValue>(wire.clone()).is_err(),
        "accepted in-memory malformed wire: {wire}"
    );
}

#[rstest]
#[case::no_leading_slash("a")]
#[case::dotted("a.b")]
#[case::uri_fragment("#/a")]
#[case::bare_escape("/a~")]
#[case::unknown_escape("/a~2")]
fn non_rfc6901_paths_are_rejected(#[case] path: &str) {
    let wire = envelope(
        json!({"a": null}),
        json!([{"path": path, "syntax": "auto", "source": "1"}]),
    );
    assert!(decode(&wire).is_err(), "accepted pointer: {path}");
}

#[rstest]
#[case::root(json!(null), "")]
#[case::empty_key(json!({"": null}), "/")]
#[case::escaped_key(json!({"a/b": null}), "/a~1b")]
fn duplicate_expression_paths_are_rejected(#[case] data: Value, #[case] path: &str) {
    let wire = envelope(
        data,
        json!([
            {"path": path, "syntax": "auto", "source": "first"}, {"path": path, "syntax": "auto", "source": "second"}
        ]),
    );
    assert!(
        decode(&wire)
            .unwrap_err()
            .to_string()
            .contains("duplicate expression path")
    );
}

#[rstest]
#[case::root(json!({"a": null}), "", "/a")]
#[case::empty_key(json!({"": {"": null}}), "/", "//")]
#[case::nested(json!({"a/b": [null]}), "/a~1b", "/a~1b/0")]
fn ancestor_paths_are_rejected_before_installation(
    #[case] data: Value,
    #[case] ancestor: &str,
    #[case] descendant: &str,
    #[values(false, true)] reverse: bool,
) {
    let mut entries = vec![
        json!({"path": ancestor, "syntax": "auto", "source": "1"}),
        json!({"path": descendant, "syntax": "auto", "source": "2"}),
    ];
    if reverse {
        entries.reverse();
    }
    let error = decode(&envelope(data, json!(entries))).unwrap_err();
    assert!(error.to_string().contains("overlapping expression paths"));
}

#[rstest]
#[case::missing_key(json!({}), "/a")]
#[case::missing_index(json!([null]), "/1")]
#[case::scalar_child(json!(null), "/a")]
#[case::root_string(json!("{{ 1 }}"), "")]
#[case::root_object(json!({}), "")]
#[case::root_list(json!([]), "")]
#[case::root_number(json!(0), "")]
#[case::root_boolean(json!(false), "")]
#[case::nested_non_null(json!({"a": "1"}), "/a")]
#[case::leading_zero(json!([null]), "/00")]
#[case::plus(json!([null]), "/+0")]
#[case::minus(json!([null]), "/-0")]
#[case::append(json!([null]), "/-")]
#[case::empty_index(json!([null]), "/")]
#[case::huge_index(json!([null]), "/184467440737095516160")]
#[case::unicode_digit(json!([null]), "/\u{660}")]
fn expression_paths_require_existing_null_slots(#[case] data: Value, #[case] path: &str) {
    let error = decode(&envelope(
        data,
        json!([{"path": path, "syntax": "auto", "source": "1"}]),
    ))
    .unwrap_err();
    assert!(error.to_string().contains("null placeholder"));
}

#[rstest]
#[case::version(r#"{"version":2,"version":2,"data":null,"expressions":[]}"#)]
#[case::data(r#"{"version":2,"data":null,"data":null,"expressions":[]}"#)]
#[case::expressions(r#"{"version":2,"data":null,"expressions":[],"expressions":[]}"#)]
#[case::path(r#"{"version":2,"data":null,"expressions":[{"path":"","path":"","syntax":"auto","source":"1"}]}"#)]
#[case::syntax(r#"{"version":2,"data":null,"expressions":[{"path":"","syntax":"auto","syntax":"template","source":"1"}]}"#)]
#[case::source(
    r#"{"version":2,"data":null,"expressions":[{"path":"","syntax":"auto","source":"1","source":"2"}]}"#
)]
#[case::property(r#"{"version":2,"data":{"a":null,"a":null},"expressions":[]}"#)]
#[case::escaped_property(r#"{"version":2,"data":{"a":null,"\u0061":null},"expressions":[]}"#)]
#[case::nested_property(r#"{"version":2,"data":[{"":null,"":null}],"expressions":[]}"#)]
fn duplicate_json_fields_are_not_silently_collapsed(#[case] wire: &str) {
    assert!(serde_json::from_slice::<AuthoredValue>(wire.as_bytes()).is_err());
}

#[test]
fn tagged_secret_data_never_constructs_a_secret() {
    let data = json!({"kind": "secret", "value": "literal", "Secret": "literal"});
    let authored = decode(&envelope(data.clone(), json!([]))).unwrap();
    assert_matches!(&authored, AuthoredValue::Object(_));
    assert_eq!(authored.first_secret_path(), None);
    assert_eq!(authored, AuthoredValue::from_data(data).unwrap());
}

const SECRET_SENTINEL: &str = "wire-secret-sentinel-4ff29";

fn assert_secret_encoding_fails<T: Serialize>(tree: &T) {
    let mut bytes = Vec::new();
    let error = serde_json::to_writer(&mut bytes, tree).unwrap_err();
    assert!(
        bytes.is_empty(),
        "secret rejection must happen before any output"
    );
    assert!(error.to_string().contains("secret-bearing values"));
    assert!(!format!("{error:?}: {error}").contains(SECRET_SENTINEL));
    assert!(serde_json::to_value(tree).is_err());
}

#[test]
fn all_phases_reject_secret_serialization_before_output() {
    let secret = SecretValue::string(SECRET_SENTINEL.to_owned());
    assert_secret_encoding_fails(&AuthoredValue::Secret(secret.clone()));
    assert_secret_encoding_fails(&CompiledValue::Secret(secret.clone()));
    assert_secret_encoding_fails(&ResolvedValue::Secret(secret.clone()));
    let mut authored = AuthoredValue::object();
    authored.insert_data("first", json!("ordinary")).unwrap();
    authored
        .insert("program", expression(SECRET_SENTINEL))
        .unwrap();
    authored
        .insert(
            "last",
            AuthoredValue::List(vec![AuthoredValue::Secret(secret.clone())]),
        )
        .unwrap();
    assert_secret_encoding_fails(&authored);
    assert_eq!(serde_json::to_value(secret).unwrap(), json!("<redacted>"));
}

#[test]
fn compiled_and_resolved_serialization_does_not_mint_their_phase_on_decode() {
    let source = "{{ 1 + 2 }}";
    let expression = Expression::new(source);
    let compiled = CompiledValue::Expression(expression.parse().unwrap().clone());
    let bytes = serde_json::to_vec(&compiled).unwrap();
    assert_eq!(
        serde_json::from_slice::<AuthoredValue>(&bytes).unwrap(),
        AuthoredValue::Expression(expression)
    );
    let resolved = ResolvedValue::from_data(json!({"$expr": "{{ literal }}"})).unwrap();
    let bytes = serde_json::to_vec(&resolved).unwrap();
    assert_eq!(
        serde_json::from_slice::<AuthoredValue>(&bytes).unwrap(),
        AuthoredValue::from_data(json!({"$expr": "{{ literal }}"})).unwrap()
    );
}

#[test]
fn maximum_depth_expression_round_trips() {
    let mut authored = expression("{{ 1 }}");
    for _ in 0..64 {
        authored = AuthoredValue::List(vec![authored]);
    }
    let bytes = serde_json::to_vec(&authored).unwrap();
    assert_eq!(
        serde_json::from_slice::<AuthoredValue>(&bytes).unwrap(),
        authored
    );
}

#[rstest]
#[case::scalar(json!(7))]
#[case::empty_object(json!({}))]
#[case::empty_list(json!([]))]
fn excessive_depth_is_rejected_by_serializer_and_both_deserializers(
    #[case] leaf: Value,
    #[values(false, true)] object: bool,
) {
    let data = nested_data(leaf.clone(), object, 65);
    let wire = envelope(data, json!([]));
    assert!(
        decode(&wire)
            .unwrap_err()
            .to_string()
            .contains("recursion_limit")
    );
    assert!(
        serde_json::from_value::<AuthoredValue>(wire)
            .unwrap_err()
            .to_string()
            .contains("recursion_limit")
    );
    let mut authored = AuthoredValue::from_data(leaf).unwrap();
    for _ in 0..65 {
        authored = if object {
            AuthoredValue::Object(std::iter::once(("child".to_owned(), authored)).collect())
        } else {
            AuthoredValue::List(vec![authored])
        };
    }
    let mut bytes = Vec::new();
    let error = serde_json::to_writer(&mut bytes, &authored).unwrap_err();
    assert!(error.to_string().contains("recursion_limit"));
    assert!(bytes.is_empty());
}

#[test]
fn expression_path_depth_is_bounded_before_lookup() {
    let path = "/0".repeat(65);
    let wire = envelope(
        json!(null),
        json!([{"path": path, "syntax": "auto", "source": "1"}]),
    );
    assert!(
        decode(&wire)
            .unwrap_err()
            .to_string()
            .contains("expression path exceeds")
    );
}

#[rstest]
#[case(json!({}))]
#[case(json!([]))]
fn scalar_constructor_cannot_hide_a_container(#[case] container: Value) {
    assert!(ScalarValue::try_from(container).is_err());
}

fn authored_strategy() -> impl Strategy<Value = AuthoredValue> {
    let scalar = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|number| json!(number)),
        any::<u64>().prop_map(|number| json!(number)),
        any::<f64>().prop_filter_map("finite JSON number", |number| {
            serde_json::Number::from_f64(number).map(Value::Number)
        }),
        any::<String>().prop_map(Value::String),
        Just(json!("{{ literal }}")),
    ]
    .prop_map(|value| AuthoredValue::from_data(value).unwrap());
    let leaf = prop_oneof![
        scalar,
        (
            any::<String>(),
            prop::sample::select(vec![
                ProgramSyntax::Auto,
                ProgramSyntax::Expression,
                ProgramSyntax::Template
            ])
        )
            .prop_map(|(source, syntax)| AuthoredValue::Expression(
                Expression::with_syntax(source, syntax)
            )),
        Just(AuthoredValue::from_data(json!({"$expr": "{{ literal }}"})).unwrap()),
    ];
    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..8).prop_map(AuthoredValue::List),
            proptest::collection::btree_map(any::<String>(), inner, 0..8)
                .prop_map(|entries| AuthoredValue::Object(entries.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn secret_free_authored_identity_survives_the_wire(authored in authored_strategy()) {
        let bytes = serde_json::to_vec(&authored).unwrap();
        let decoded: AuthoredValue = serde_json::from_slice(&bytes).unwrap();
        prop_assert_eq!(&decoded, &authored);
        prop_assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
    }
}
