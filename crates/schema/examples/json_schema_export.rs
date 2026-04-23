//! Export a validated schema to JSON Schema Draft 2020-12 (requires the `schemars` feature).
//!
//! Run:
//! `cargo run -p nebula-schema --example json_schema_export --features schemars`

use nebula_schema::{Field, FieldKey, Schema};
use serde_json::Value;

fn main() {
    let key = FieldKey::new("name").expect("valid key");
    let schema = Schema::builder()
        .add(Field::string(key).required().label("Name").no_expression())
        .build()
        .expect("lint");

    let exported = schema.json_schema().expect("export to JSON Schema");
    let json: Value = serde_json::to_value(&exported).expect("schemars::Schema serializes");

    assert_eq!(
        json["$schema"],
        Value::String("https://json-schema.org/draft/2020-12/schema".to_owned())
    );
    assert_eq!(json["type"], "object");
    eprintln!("OK: JSON Schema export: {}", json["$schema"]);
}
