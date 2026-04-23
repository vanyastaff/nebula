//! Build a schema with the builder API, parse `FieldValues` from JSON, and run
//! `ValidSchema::validate` to obtain a `ValidValues` proof token.
//!
//! Run:
//! `cargo run -p nebula-schema --example builder_validate`

use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

fn main() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .add(
            Field::number(field_key!("retries"))
                .default(json!(3))
                .label("Retries"),
        )
        .build()
        .expect("structural lint should pass");

    let values = FieldValues::from_json(json!({"name": "demo", "retries": 1}))
        .expect("strict key validation should accept this object");

    let valid = schema
        .validate(&values)
        .expect("field values should satisfy the schema");

    assert!(valid.warnings().is_empty());
    eprintln!("OK: validated {} top-level field(s)", schema.fields().len());
}
