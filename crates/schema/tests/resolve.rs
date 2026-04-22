//! Integration tests for `ValidValues::resolve` (Task 23).
//!
//! Covers: fast path, expression evaluation, evaluation failure,
//! nested structures, and post-resolve literal passthrough.

use nebula_schema::*;
use serde_json::json;

#[derive(Debug, serde::Deserialize, PartialEq)]
struct Person {
    name: String,
}

// ── Stub ExpressionContext ────────────────────────────────────────────────────

/// Returns a constant value for every expression.
struct ConstCtx(serde_json::Value);

#[async_trait::async_trait]
impl ExpressionContext for ConstCtx {
    async fn evaluate(&self, _ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
        Ok(self.0.clone())
    }
}

/// Returns values based on expression source fragments.
struct RoutingCtx;

#[async_trait::async_trait]
impl ExpressionContext for RoutingCtx {
    async fn evaluate(&self, ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
        if ast.source().contains("$bad_str") {
            return Ok(json!(123));
        }
        if ast.source().contains("$ok_str") {
            return Ok(json!("ok"));
        }
        if ast.source().contains("$bad_item") {
            return Ok(json!(999));
        }
        if ast.source().contains("$ok_num") {
            return Ok(json!(42));
        }
        Ok(json!(null))
    }
}

/// Always fails with `expression.runtime`.
struct FailCtx;

#[async_trait::async_trait]
impl ExpressionContext for FailCtx {
    async fn evaluate(&self, _ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
        Err(ValidationError::builder("expression.runtime")
            .message("evaluation failed")
            .build())
    }
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

    let values = FieldValues::from_json(json!({"flag": true})).unwrap();
    let validated = schema.validate(&values).unwrap();
    let resolved = validated.resolve(&ConstCtx(json!(null))).await.unwrap();

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

    let values = FieldValues::from_json(json!({"n": {"$expr": "{{ $x }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();

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

    let values = FieldValues::from_json(json!({"s": "hello"})).unwrap();
    let validated = schema.validate(&values).unwrap();

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

    let values = FieldValues::from_json(json!({"x": {"$expr": "{{ $bad }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();

    let report = validated.resolve(&FailCtx).await.unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.runtime"),
        "expected expression.runtime, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_returns_expression_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"x": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();

    let report = validated.resolve(&ConstCtx(json!(123))).await.unwrap_err();
    assert!(
        report
            .errors()
            .any(|e| e.code == "expression.type_mismatch"),
        "expected expression.type_mismatch, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_in_nested_object_is_remapped() {
    let schema = Schema::builder()
        .add(Field::object(field_key!("user")).add(Field::string(field_key!("name"))))
        .build()
        .unwrap();

    let values =
        FieldValues::from_json(json!({"user": {"name": {"$expr": "{{ $bad_str }}"}}})).unwrap();
    let validated = schema.validate(&values).unwrap();

    let report = validated.resolve(&RoutingCtx).await.unwrap_err();
    assert!(
        report.errors().any(|e| {
            e.code == "expression.type_mismatch" && e.path == FieldPath::parse("user.name").unwrap()
        }),
        "expected expression.type_mismatch at user.name, got: {:?}",
        report
            .errors()
            .map(|e| (e.code.clone(), e.path.to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_in_list_item_is_remapped() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("tags")).item(Field::string(field_key!("_item"))))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "tags": [{"$expr": "{{ $bad_item }}"}]
    }))
    .unwrap();
    let validated = schema.validate(&values).unwrap();

    let report = validated.resolve(&RoutingCtx).await.unwrap_err();
    assert!(
        report.errors().any(|e| {
            e.code == "expression.type_mismatch" && e.path == FieldPath::parse("tags[0]").unwrap()
        }),
        "expected expression.type_mismatch at tags[0], got: {:?}",
        report
            .errors()
            .map(|e| (e.code.clone(), e.path.to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn expression_type_mismatch_remap_is_scoped_to_failing_sibling() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("a")))
        .add(Field::number(field_key!("b")))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "a": {"$expr": "{{ $bad_str }}"},
        "b": {"$expr": "{{ $ok_num }}"}
    }))
    .unwrap();
    let validated = schema.validate(&values).unwrap();

    let report = validated.resolve(&RoutingCtx).await.unwrap_err();
    let mismatch_paths: Vec<String> = report
        .errors()
        .filter(|e| e.code == "expression.type_mismatch")
        .map(|e| e.path.to_string())
        .collect();
    assert_eq!(
        mismatch_paths,
        vec!["a".to_string()],
        "expected remap only for failing sibling, got: {:?}",
        report
            .errors()
            .map(|e| (e.code.clone(), e.path.to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        !report.errors().any(|e| e.code == "type_mismatch"),
        "raw type_mismatch should have been remapped, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
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

    let values = FieldValues::from_json(json!({
        "user": {
            "name": {"$expr": "{{ $name }}"},
            "email": "static@example.com"
        }
    }))
    .unwrap();

    let validated = schema.validate(&values).unwrap();
    let resolved = validated.resolve(&ConstCtx(json!("Alice"))).await.unwrap();

    let name = resolved
        .values()
        .get_path(&FieldPath::parse("user.name").unwrap());
    let email = resolved
        .values()
        .get_path(&FieldPath::parse("user.email").unwrap());
    assert_eq!(name, Some(&FieldValue::Literal(json!("Alice"))));
    assert_eq!(
        email,
        Some(&FieldValue::Literal(json!("static@example.com")))
    );
}

// ── List with expressions ─────────────────────────────────────────────────────

#[tokio::test]
async fn list_items_with_expressions_resolve() {
    let schema = Schema::builder()
        .add(Field::list(field_key!("tags")).item(Field::string(field_key!("_item"))))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "tags": [
            "literal-tag",
            {"$expr": "{{ $dynamic_tag }}"}
        ]
    }))
    .unwrap();

    let validated = schema.validate(&values).unwrap();
    let ctx = ConstCtx(json!("evaluated-tag"));
    let resolved = validated.resolve(&ctx).await.unwrap();

    let first = resolved
        .values()
        .get_path(&FieldPath::parse("tags[0]").unwrap());
    let second = resolved
        .values()
        .get_path(&FieldPath::parse("tags[1]").unwrap());
    assert_eq!(first, Some(&FieldValue::Literal(json!("literal-tag"))));
    assert_eq!(second, Some(&FieldValue::Literal(json!("evaluated-tag"))));
}

// ── Multiple expressions — all resolved in one pass ───────────────────────────

#[tokio::test]
async fn multiple_expressions_all_resolve() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("a")))
        .add(Field::string(field_key!("b")))
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({
        "a": {"$expr": "{{ $x }}"},
        "b": {"$expr": "{{ $y }}"}
    }))
    .unwrap();

    let validated = schema.validate(&values).unwrap();
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

    let values = FieldValues::from_json(json!({"name": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();
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

    let values = FieldValues::from_json(json!({"name": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();
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

    let values = FieldValues::from_json(json!({"name": {"$expr": "{{ $n }}"}})).unwrap();
    let validated = schema.validate(&values).unwrap();
    let resolved = validated.resolve(&ConstCtx(json!("Bob"))).await.unwrap();

    let err = resolved.into_typed::<u64>().unwrap_err();
    assert_eq!(err.code, "type_mismatch");
}

#[tokio::test]
async fn secret_field_promotes_and_resolved_get_sanitizes_json() {
    let schema = Schema::builder()
        .add(Field::secret(field_key!("api_key")).required())
        .build()
        .unwrap();

    let values = FieldValues::from_json(json!({"api_key": "sekrit"})).unwrap();
    let valid = schema.validate(&values).unwrap();
    let resolved = valid.resolve(&ConstCtx(json!(null))).await.unwrap();

    assert!(resolved.get(&field_key!("api_key")).is_none());
    assert!(matches!(
        resolved.lookup(&field_key!("api_key")),
        ResolvedLookup::Secret(_)
    ));
    assert!(matches!(
        resolved.lookup(&field_key!("missing")),
        ResolvedLookup::Missing
    ));
    let sec = resolved.get_secret(&field_key!("api_key")).expect("secret");
    let SecretValue::String(s) = sec else {
        panic!("expected string secret");
    };
    assert_eq!(s.expose(), "sekrit");

    let wire = resolved.values().to_json();
    let obj = wire.as_object().expect("object");
    assert_eq!(obj.get("api_key"), Some(&json!("<redacted>")));
}
