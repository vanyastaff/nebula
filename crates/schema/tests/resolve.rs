//! Integration tests for `ValidValues::resolve` (Task 23).
//!
//! Covers: fast path, expression evaluation, evaluation failure,
//! nested structures, and post-resolve literal passthrough.

use nebula_schema::*;
use serde_json::json;

// ── Stub ExpressionContext ────────────────────────────────────────────────────

/// Returns a constant value for every expression.
struct ConstCtx(serde_json::Value);

#[async_trait::async_trait]
impl ExpressionContext for ConstCtx {
    async fn evaluate(&self, _ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
        Ok(self.0.clone())
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

    // name was an expression — resolved to "Alice"
    let user = resolved
        .values()
        .get_path(&FieldPath::parse("user").unwrap());
    assert!(user.is_some(), "user field should be present");
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

    // The resolved values should have the list field.
    let tags = resolved.values().get(&field_key!("tags"));
    assert!(
        tags.is_some(),
        "tags field should be present after resolution"
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
