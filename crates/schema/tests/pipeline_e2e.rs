//! End-to-end integration: build a schema, ingest `FieldValues`, validate, then resolve
//! with a stub [`nebula_schema::ExpressionContext`].
//!
//! This mirrors the `examples/async_resolve` flow but is CI-friendly and uses structured
//! assertions instead of `main`.

use nebula_schema::{
    ExpressionAst, ExpressionContext, Field, FieldValues, Schema, ValidationError, field_key,
};
use serde_json::json;

struct Ctx(serde_json::Value);

#[async_trait::async_trait]
impl ExpressionContext for Ctx {
    async fn evaluate(&self, _ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn e2e_happy_path_validate_then_resolve() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .add(Field::number(field_key!("count")))
        .build()
        .expect("schema lints");

    let values = FieldValues::from_json(json!({
        "name": "e2e",
        "count": { "$expr": "{{ x }}" },
    }))
    .expect("ingest");

    let valid = schema.validate(&values).expect("valid values");
    let resolved = valid.resolve(&Ctx(json!(7.0))).await.expect("resolve");
    assert_eq!(resolved.get(&field_key!("count")), Some(&json!(7.0)));
}

#[tokio::test]
async fn e2e_fast_path_no_expressions_uses_booleans() {
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("flag")))
        .build()
        .expect("build");

    assert!(!schema.flags().uses_expressions);

    let values = FieldValues::from_json(json!({ "flag": true })).expect("ingest");
    let valid = schema.validate(&values).expect("valid");
    let resolved = valid.resolve(&Ctx(json!(null))).await.expect("resolve");
    assert_eq!(resolved.get(&field_key!("flag")), Some(&json!(true)));
}
