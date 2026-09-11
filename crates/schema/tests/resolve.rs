//! Integration tests for `ValidValues::resolve`.
//!
//! Covers: fast path, expression evaluation, evaluation failure,
//! nested structures, and post-resolve literal passthrough.

use std::{
    assert_matches,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use nebula_schema::*;
use serde_json::json;

fn nested_list(mut leaf: serde_json::Value, depth: u8) -> serde_json::Value {
    for _ in 0..depth {
        leaf = json!([leaf]);
    }
    leaf
}

#[derive(Debug, serde::Deserialize, PartialEq)]
struct Person {
    name: String,
}

#[derive(Debug, PartialEq)]
struct SecretWrapper(String);

impl<'de> serde::Deserialize<'de> for SecretWrapper {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

#[derive(Debug, serde::Deserialize, PartialEq)]
struct ApiCredential {
    api_key: SecretWrapper,
}

// ── Stub ExpressionContext ────────────────────────────────────────────────────

/// Returns a constant value for every expression.
struct ConstCtx(serde_json::Value);

impl ExpressionContext for ConstCtx {
    fn evaluate<'a>(&'a self, _program: &'a CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

/// Returns values based on expression source fragments.
struct RoutingCtx;

impl ExpressionContext for RoutingCtx {
    fn evaluate<'a>(&'a self, program: &'a CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move {
            if program.source().contains("$bad_str") {
                return Ok(json!(123));
            }
            if program.source().contains("$ok_str") {
                return Ok(json!("ok"));
            }
            if program.source().contains("$bad_item") {
                return Ok(json!(999));
            }
            if program.source().contains("$ok_num") {
                return Ok(json!(42));
            }
            Ok(json!(null))
        })
    }
}

/// Always fails with `expression.runtime`.
struct FailCtx;

impl ExpressionContext for FailCtx {
    fn evaluate<'a>(&'a self, _program: &'a CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move {
            Err(ValidationError::builder("expression.runtime")
                .message("evaluation failed")
                .build())
        })
    }
}

#[tokio::test]
async fn literal_resolution_accepts_the_authored_depth_boundary() {
    let input = nested_list(json!(null), MAX_VALUE_DEPTH);
    let resolved = ValidSchema::any()
        .validate(AuthoredValue::from_data(input.clone()).unwrap())
        .unwrap()
        .resolve(&FailCtx)
        .await
        .unwrap();

    assert_eq!(resolved.into_json(), input);
}

struct CountingLargeResultCtx {
    evaluations: Arc<AtomicUsize>,
    result: serde_json::Value,
}

impl ExpressionContext for CountingLargeResultCtx {
    fn evaluate<'a>(&'a self, _program: &'a CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move {
            self.evaluations.fetch_add(1, Ordering::Relaxed);
            Ok(self.result.clone())
        })
    }
}

#[tokio::test]
async fn aggregate_result_budget_stops_before_resolving_later_expressions() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("items")).item(Field::string(field_key!("item"))))
        .build()
        .unwrap();
    let authored = AuthoredValue::from_template_json(json!({
        "items": [
            {"$expr": "{{ $first }}"},
            {"$expr": "{{ $second }}"},
            {"$expr": "{{ $third }}"}
        ]
    }))
    .unwrap();
    let evaluations = Arc::new(AtomicUsize::new(0));
    let context = CountingLargeResultCtx {
        evaluations: Arc::clone(&evaluations),
        result: json!("x".repeat(MAX_VALUE_TEXT_BYTES / 2 + 1)),
    };

    let report = schema
        .validate(authored)
        .unwrap()
        .resolve(&context)
        .await
        .unwrap_err();

    assert_eq!(evaluations.load(Ordering::Relaxed), 2);
    assert!(
        report
            .errors()
            .any(|error| error.code() == "value.limit_exceeded")
    );
}

// ── Fast path (no expressions) ────────────────────────────────────────────────

#[tokio::test]
async fn fast_path_no_expressions() {
    // Schema where all fields are ExpressionMode::Forbidden — uses_expressions = false.
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("flag")))
        .build()
        .unwrap();

    // Confirm flag.
    assert!(
        !schema.flags().uses_expressions,
        "boolean is expression-forbidden → uses_expressions must be false"
    );

    let values = AuthoredValue::from_data(json!({"flag": true})).unwrap();
    let validated = schema.validate(values).unwrap();
    let resolved = validated.resolve(&FailCtx).await.unwrap();

    assert_eq!(resolved.get(&field_key!("flag")), Some(&json!(true)));
    assert!(resolved.warnings().is_empty());
}

// ── Expression evaluates and replaces with literal ────────────────────────────

#[tokio::test]
async fn expression_resolves_to_literal() {
    let schema = Schema::builder()
        .add(Field::number(field_key!("n")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({"n": {"$expr": "{{ $x }}"}})).unwrap();
    let validated = schema.validate(values).unwrap();

    let ctx = ConstCtx(json!(42.0));
    let resolved = validated.resolve(&ctx).await.unwrap();

    assert_eq!(resolved.get(&field_key!("n")), Some(&json!(42.0)));
}

// ── Literal values pass through unchanged ─────────────────────────────────────

#[tokio::test]
async fn literal_values_pass_through() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("s")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_data(json!({"s": "hello"})).unwrap();
    let validated = schema.validate(values).unwrap();

    let resolved = validated
        .resolve(&ConstCtx(json!("ignored")))
        .await
        .unwrap();

    assert_eq!(resolved.get(&field_key!("s")), Some(&json!("hello")));
}

// ── Expression evaluation failure → expression.runtime error ─────────────────

#[tokio::test]
async fn expression_evaluation_failure_returns_report() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({"x": {"$expr": "{{ $bad }}"}})).unwrap();
    let validated = schema.validate(values).unwrap();

    let report = validated.resolve(&FailCtx).await.unwrap_err();
    assert!(
        report.errors().any(|e| e.code() == "expression.runtime"),
        "expected expression.runtime, got: {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_returns_expression_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({"x": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(values).unwrap();

    let report = validated.resolve(&ConstCtx(json!(123))).await.unwrap_err();
    assert!(
        report
            .errors()
            .any(|e| e.code() == "expression.type_mismatch"),
        "expected expression.type_mismatch, got: {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code() == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_in_nested_object_is_remapped() {
    let schema = Schema::builder()
        .add(Field::object(field_key!("user")).add(Field::string(field_key!("name"))))
        .build()
        .unwrap();

    let values =
        AuthoredValue::from_template_json(json!({"user": {"name": {"$expr": "{{ $bad_str }}"}}}))
            .unwrap();
    let validated = schema.validate(values).unwrap();

    let report = validated.resolve(&RoutingCtx).await.unwrap_err();
    assert!(
        report.errors().any(|e| {
            e.code() == "expression.type_mismatch"
                && e.path() == &ValuePath::parse("/user/name").unwrap()
        }),
        "expected expression.type_mismatch at user.name, got: {:?}",
        report
            .errors()
            .map(|e| (e.code(), e.path().to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code() == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_in_list_item_is_remapped() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("tags")).item(Field::string(field_key!("_item"))))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({
        "tags": [{"$expr": "{{ $bad_item }}"}]
    }))
    .unwrap();
    let validated = schema.validate(values).unwrap();

    let report = validated.resolve(&RoutingCtx).await.unwrap_err();
    assert!(
        report.errors().any(|e| {
            e.code() == "expression.type_mismatch"
                && e.path() == &ValuePath::parse("/tags/0").unwrap()
        }),
        "expected expression.type_mismatch at tags[0], got: {:?}",
        report
            .errors()
            .map(|e| (e.code(), e.path().to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code() == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_remap_is_scoped_to_failing_sibling() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("a")))
        .add(Field::number(field_key!("b")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({
        "a": {"$expr": "{{ $bad_str }}"},
        "b": {"$expr": "{{ $ok_num }}"}
    }))
    .unwrap();
    let validated = schema.validate(values).unwrap();

    let report = validated.resolve(&RoutingCtx).await.unwrap_err();
    let mismatch_paths: Vec<String> = report
        .errors()
        .filter(|e| e.code() == "expression.type_mismatch")
        .map(|e| e.path().to_string())
        .collect();
    assert_eq!(
        mismatch_paths,
        vec!["/a".to_string()],
        "expected remap only for failing sibling, got: {:?}",
        report
            .errors()
            .map(|e| (e.code(), e.path().to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code() == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

// ── Nested object with expressions ────────────────────────────────────────────

#[tokio::test]
async fn nested_object_expressions_resolve() {
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("user"))
                .add(Field::string(field_key!("name")))
                .add(Field::string(field_key!("email"))),
        )
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({
        "user": {
            "name": {"$expr": "{{ $name }}"},
            "email": "static@example.com"
        }
    }))
    .unwrap();

    let validated = schema.validate(values).unwrap();
    let resolved = validated.resolve(&ConstCtx(json!("Alice"))).await.unwrap();

    let name = resolved
        .values()
        .get_path(&ValuePath::parse("/user/name").unwrap());
    let email = resolved
        .values()
        .get_path(&ValuePath::parse("/user/email").unwrap());
    assert_eq!(name.and_then(ValueTree::as_literal), Some(&json!("Alice")));
    assert_eq!(
        email.and_then(ValueTree::as_literal),
        Some(&json!("static@example.com"))
    );
}

// ── List with expressions ─────────────────────────────────────────────────────

#[tokio::test]
async fn list_items_with_expressions_resolve() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("tags")).item(Field::string(field_key!("_item"))))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({
        "tags": [
            "literal-tag",
            {"$expr": "{{ $dynamic_tag }}"}
        ]
    }))
    .unwrap();

    let validated = schema.validate(values).unwrap();
    let ctx = ConstCtx(json!("evaluated-tag"));
    let resolved = validated.resolve(&ctx).await.unwrap();

    let first = resolved
        .values()
        .get_path(&ValuePath::parse("/tags/0").unwrap());
    let second = resolved
        .values()
        .get_path(&ValuePath::parse("/tags/1").unwrap());
    assert_eq!(
        first.and_then(ValueTree::as_literal),
        Some(&json!("literal-tag"))
    );
    assert_eq!(
        second.and_then(ValueTree::as_literal),
        Some(&json!("evaluated-tag"))
    );
}

// ── Multiple expressions — all resolved in one pass ───────────────────────────

#[tokio::test]
async fn multiple_expressions_all_resolve() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("a")))
        .add(Field::string(field_key!("b")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({
        "a": {"$expr": "{{ $x }}"},
        "b": {"$expr": "{{ $y }}"}
    }))
    .unwrap();

    let validated = schema.validate(values).unwrap();
    let resolved = validated
        .resolve(&ConstCtx(json!("resolved")))
        .await
        .unwrap();

    assert_eq!(resolved.get(&field_key!("a")), Some(&json!("resolved")));
    assert_eq!(resolved.get(&field_key!("b")), Some(&json!("resolved")));
}

// ── into_json / into_typed ────────────────────────────────────────────────────

#[tokio::test]
async fn into_json_works_after_resolution() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({"name": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(values).unwrap();
    let resolved = validated.resolve(&ConstCtx(json!("Bob"))).await.unwrap();

    let out = resolved.into_json();
    assert_eq!(out, json!({"name": "Bob"}));
}

#[tokio::test]
async fn into_typed_deserializes_successfully() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({"name": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(values).unwrap();
    let resolved = validated.resolve(&ConstCtx(json!("Bob"))).await.unwrap();

    let typed: Person = resolved.into_typed().unwrap();
    assert_eq!(
        typed,
        Person {
            name: "Bob".to_owned()
        }
    );
}

#[tokio::test]
async fn into_typed_returns_type_mismatch_on_deserialize_failure() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")))
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({"name": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(values).unwrap();
    let resolved = validated.resolve(&ConstCtx(json!("Bob"))).await.unwrap();

    let err = resolved.into_typed::<u64>().unwrap_err();
    assert_eq!(err.code(), "type_mismatch");
}

#[tokio::test]
async fn secret_field_promotes_and_resolved_get_sanitizes_json() {
    let schema = Schema::builder()
        .add(Field::secret(field_key!("api_key")).required())
        .build()
        .unwrap();

    let values = AuthoredValue::from_data(json!({"api_key": "sekrit"})).unwrap();
    let valid = schema.validate(values).unwrap();
    let resolved = valid.resolve(&ConstCtx(json!(null))).await.unwrap();

    assert!(resolved.get(&field_key!("api_key")).is_none());
    assert_matches!(
        resolved.lookup(&field_key!("api_key")),
        ResolvedLookup::Secret(_)
    );
    assert_matches!(
        resolved.lookup(&field_key!("missing")),
        ResolvedLookup::Missing
    );
    let sec = resolved.get_secret(&field_key!("api_key")).expect("secret");
    let SecretValue::String(s) = sec else {
        panic!("expected string secret");
    };
    assert_eq!(s.expose(), "sekrit");

    let wire = resolved.values().to_json();
    let obj = wire.as_object().expect("object");
    assert_eq!(obj.get("api_key"), Some(&json!("<redacted>")));
}

#[tokio::test]
async fn into_typed_rejects_secret_material_by_default() {
    let schema = Schema::builder()
        .add(Field::secret(field_key!("api_key")).required())
        .build()
        .unwrap();

    let values = AuthoredValue::from_data(json!({"api_key": "sekrit"})).unwrap();
    let valid = schema.validate(values).unwrap();
    let resolved = valid.resolve(&ConstCtx(json!(null))).await.unwrap();

    let err = resolved.into_typed::<ApiCredential>().unwrap_err();
    assert_eq!(err.code(), "type_mismatch");
    assert_eq!(err.path().to_string(), "/api_key");
    assert!(
        err.message().contains("explicit secret access"),
        "expected explicit opt-in guidance, got: {}",
        err.message()
    );
}

#[tokio::test]
async fn resolve_promotes_mode_object_envelope_secrets_via_default_variant() {
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth"))
                .variant(
                    "token",
                    "Token",
                    Field::object(field_key!("payload")).add(Field::secret(field_key!("api_key"))),
                )
                .default_variant("token")
                .required(),
        )
        .build()
        .unwrap();

    let values = AuthoredValue::from_data(json!({
        "auth": {
            "value": {
                "api_key": "sekrit"
            }
        }
    }))
    .unwrap();
    let valid = schema.validate(values).unwrap();
    let resolved = valid.resolve(&ConstCtx(json!(null))).await.unwrap();

    let secret_path = ValuePath::parse("/auth/value/api_key").unwrap();
    let secret = resolved
        .values()
        .get_path(&secret_path)
        .expect("secret path");
    assert_matches!(secret, ValueTree::Secret(_));

    let wire = resolved.values().to_json();
    assert_eq!(
        wire.pointer("/auth/value/api_key"),
        Some(&json!("<redacted>"))
    );
}

#[tokio::test]
async fn resolved_expression_object_folds_aliases_and_protects_secrets() {
    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Credentials {
        creds: ApiCredential,
    }

    let schema = Schema::builder()
        .add(
            Field::object(field_key!("creds"))
                .expression_mode(ExpressionMode::Allowed)
                .add(
                    Field::secret(field_key!("api_key"))
                        .read_alias("token_alias")
                        .unwrap(),
                ),
        )
        .build()
        .unwrap();

    let values = AuthoredValue::from_template_json(json!({
        "creds": {"$expr": "{{ $resolve }}"}
    }))
    .unwrap();
    let valid = schema.validate(values).unwrap();
    let resolved = valid
        .resolve(&ConstCtx(json!({"token_alias": "PLAINTEXT_SECRET"})))
        .await
        .unwrap();

    let credentials = resolved.values().get("creds").unwrap().as_object().unwrap();
    assert_eq!(credentials.len(), 1);
    assert!(!credentials.contains_key("token_alias"));
    let secret_path = ValuePath::parse("/creds/api_key").unwrap();
    let Some(ValueTree::Secret(SecretValue::String(secret))) = resolved.get_path(&secret_path)
    else {
        panic!("resolved alias must be promoted to a secret at the canonical path");
    };
    assert_eq!(secret.expose(), "PLAINTEXT_SECRET");
    assert!(resolved.get(&field_key!("creds")).is_none());
    assert_matches!(
        resolved.lookup(&field_key!("creds")),
        ResolvedLookup::Complex(_)
    );
    assert_eq!(
        resolved.values().to_json(),
        json!({"creds": {"api_key": "<redacted>"}})
    );
    assert_eq!(resolved.to_wire_json(), json!({"creds": {}}));
    let data_debug = format!("{:?}", resolved.values());
    assert!(
        !data_debug.contains("token_alias"),
        "read alias survived in the resolved data tree"
    );
    let serialization_error = serde_json::to_string(resolved.values()).unwrap_err();
    assert!(serialization_error.to_string().contains("secret-bearing"));
    for output in [
        format!("{resolved:?}"),
        data_debug,
        serialization_error.to_string(),
        resolved.clone().into_json().to_string(),
        resolved.to_wire_json().to_string(),
    ] {
        assert!(
            !output.contains("PLAINTEXT_SECRET"),
            "secret escaped a redacted view"
        );
    }

    let error = resolved.clone().into_typed::<Credentials>().unwrap_err();
    assert_eq!(error.code(), "type_mismatch");
    assert_eq!(error.path(), &secret_path);
    assert!(!format!("{error:?}").contains("PLAINTEXT_SECRET"));
    let typed: Credentials = resolved.into_typed_exposing_secrets().unwrap();
    assert_eq!(
        typed,
        Credentials {
            creds: ApiCredential {
                api_key: SecretWrapper("PLAINTEXT_SECRET".to_owned()),
            },
        }
    );
}

#[rstest::rstest]
#[case::declared_child(
    Field::object(field_key!("payload"))
        .no_expression()
        .add(Field::string(field_key!("child")))
        .into(),
    json!({"payload": {"child": {"$expr": "{{ $x }}"}}}),
    "/payload/child",
)]
#[case::hidden_extra(
    Field::object(field_key!("payload")).no_expression().into(),
    json!({"payload": {"extra": [{"$expr": "{{ $x }}"}]}}),
    "/payload/extra/0",
)]
#[case::declared_list_item(
    Field::list(field_key!("payload"))
        .no_expression()
        .item(Field::string(field_key!("item")))
        .into(),
    json!({"payload": [{"$expr": "{{ $x }}"}]}),
    "/payload/0",
)]
#[case::opaque_field(
    serde_json::from_value(json!({"type": "future_widget", "key": "payload"})).unwrap(),
    json!({"payload": {"extra": [{"$expr": "{{ $x }}"}]}}),
    "/payload/extra/0",
)]
fn forbidden_subtrees_reject_expressions_in_declared_children_and_hidden_data(
    #[case] field: Field,
    #[case] input: serde_json::Value,
    #[case] pointer: &str,
) {
    let schema = Schema::builder().add(field).build().unwrap();
    let values = AuthoredValue::from_template_json(input).unwrap();
    let report = schema
        .validate(values)
        .expect_err("subtree forbids expressions");
    let errors: Vec<_> = report
        .errors()
        .map(|error| (error.code(), error.path().to_string()))
        .collect();
    assert_eq!(errors, [("expression.forbidden", pointer.to_owned())]);
}
