//! Use `#[derive(Schema)]` so the Rust type and the Nebula field list stay in sync.
//! `HasSchema::schema()` returns the same `ValidSchema` the engine would load from wire.
//!
//! Run:
//! `cargo run -p nebula-schema --example derive_config`

use nebula_schema::{FieldValues, HasSchema, Schema};
use serde::Deserialize;
use serde_json::json;

#[derive(Schema, Deserialize, Debug)]
struct DemoConfig {
    /// Shown in generated schema metadata.
    #[param(label = "Display title")]
    title: String,
}

fn main() {
    let schema = DemoConfig::schema();
    assert_eq!(
        schema.fields().len(),
        1,
        "one struct field → one top-level field"
    );

    let sample = json!({"title": "hello"});
    let values = FieldValues::from_json(sample.clone()).expect("json");
    schema
        .validate(&values)
        .expect("valid against derived schema");
    let cfg: DemoConfig = serde_json::from_value(sample).expect("round-trip");
    eprintln!("OK: derived schema validates title={}", cfg.title);
}
