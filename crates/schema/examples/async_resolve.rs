//! Full proof-token pipeline: `ValidSchema` → `ValidValues` → `ResolvedValues` using a
//! tiny [`ExpressionContext`](nebula_schema::ExpressionContext) stub.
//!
//! Run:
//! `cargo run -p nebula-schema --example async_resolve`

#![expect(
    clippy::print_stderr,
    reason = "example: errors are reported to stderr"
)]

use nebula_schema::{
    AuthoredValue, CompiledProgram, EvalFuture, ExpressionContext, Field, Schema, field_key,
};
use serde_json::json;

/// Returns the same JSON for every expression — enough to resolve `{{ $x }}` shapes in tests.
struct ConstJson(serde_json::Value);

impl ExpressionContext for ConstJson {
    fn evaluate<'a>(&'a self, _program: &'a CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

#[tokio::main]
async fn main() {
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("enabled")))
        .add(Field::number(field_key!("n")))
        .build()
        .expect("lint");

    // Literal + expression wire for `n`.
    let values = AuthoredValue::from_template_json(json!({
        "enabled": true,
        "n": {"$expr": "{{ cost }}"},
    }))
    .expect("authored input");

    let valid = schema.validate(values).expect("validate");
    let ctx = ConstJson(json!(42.0));
    let resolved = valid.resolve(&ctx).await.expect("resolve");

    assert_eq!(resolved.get(&field_key!("n")), Some(&json!(42.0)));
    eprintln!("OK: resolved numeric field via expression context");
}
