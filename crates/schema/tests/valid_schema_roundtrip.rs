//! Round-trip test for `ValidSchema` serialization.
//!
//! Guards the wire-protocol contract used by `nebula-plugin-sdk` / `nebula-sandbox`:
//! schemas declared by plugin authors must survive a JSON round-trip without
//! losing field shape.

use nebula_schema::{Field, Schema, ValidSchema, field_key};

#[test]
fn valid_schema_json_roundtrip_preserves_fields() {
    let original = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .add(Field::number(field_key!("age")))
        .build()
        .unwrap();

    let json = serde_json::to_string(&original).expect("serialize");
    let decoded: ValidSchema = serde_json::from_str(&json).expect("deserialize");

    // `ValidSchema: PartialEq` compares `fields` and `root_rules` (not `Arc` identity).
    // `index`/`flags` are recomputed by the builder and are not part of the wire contract.
    assert_eq!(original, decoded);
}

#[test]
fn valid_schema_empty_roundtrip() {
    let empty = Schema::builder().build().unwrap();

    let json = serde_json::to_string(&empty).expect("serialize");
    let decoded: ValidSchema = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(empty, decoded);

    // Lock wire shape: protocol-version 3 consumers rely on this exact JSON
    // for zero-field schemas.
    assert_eq!(json, r#"{"fields":[]}"#);
}
