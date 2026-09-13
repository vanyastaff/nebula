//! Build a schema, ingest JSON data as `AuthoredValue`, and run
//! `ValidSchema::validate` to obtain prepared `ValidValues`.
//!
//! Run:
//! `cargo run -p nebula-schema --example builder_validate`

#![expect(
    clippy::print_stderr,
    reason = "example: errors are reported to stderr"
)]

use nebula_schema::{AuthoredValue, Field, Schema, field_key};
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

    let values =
        AuthoredValue::from_data(json!({"name": "demo", "retries": 1})).expect("bounded JSON data");

    let valid = schema
        .validate(values)
        .expect("field values should satisfy the schema");

    assert!(valid.warnings().is_empty());
    assert_eq!(
        valid.values().to_json(),
        json!({"name": "demo", "retries": 1})
    );
    eprintln!("OK: validated {} top-level field(s)", schema.fields().len());
}
