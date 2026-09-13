//! Use `#[derive(Schema)]` so the Rust type and the Nebula field list stay in sync.
//! `HasSchema::schema()` checks the derived definition before returning a `ValidSchema`.
//!
//! Run:
//! `cargo run -p nebula-schema --example derive_config`

#![expect(
    clippy::print_stderr,
    reason = "example: errors are reported to stderr"
)]

use nebula_schema::{AuthoredValue, HasSchema, Schema};
use serde::Deserialize;
use serde_json::json;

#[derive(Schema, Deserialize, Debug)]
struct DemoConfig {
    /// Shown in generated schema metadata.
    #[field(label = "Display title")]
    title: String,
}

fn main() {
    let schema = DemoConfig::schema().expect("derived schema lints");
    assert_eq!(
        schema.fields().len(),
        1,
        "one struct field → one top-level field"
    );

    let sample = json!({"title": "hello"});
    let values = AuthoredValue::from_data(sample).expect("json");
    let resolved = schema
        .validate(values)
        .expect("valid against derived schema")
        .resolve_data()
        .expect("literal configuration completes");
    let cfg: DemoConfig = resolved.into_typed().expect("round-trip");
    assert_eq!(cfg.title, "hello");
    eprintln!("OK: derived schema validates title={}", cfg.title);
}
