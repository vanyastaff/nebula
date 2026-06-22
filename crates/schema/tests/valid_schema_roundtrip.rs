//! Round-trip test for `ValidSchema` serialization.
//!
//! Guards the wire-protocol contract used by `nebula-plugin-sdk`:
//! schemas declared by plugin authors must survive a JSON round-trip without
//! losing field shape.

use nebula_schema::{Field, Schema, SchemaKind, ValidSchema, field_key};

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
    // for zero-field schemas. The empty *record* must stay `kind`-less so it
    // round-trips unchanged through readers that pre-date `SchemaKind`.
    assert_eq!(json, r#"{"fields":[]}"#);
    assert_eq!(empty.kind(), SchemaKind::Record);
}

#[test]
fn valid_schema_any_roundtrip_preserves_kind() {
    let any = ValidSchema::any();

    let json = serde_json::to_string(&any).expect("serialize");
    // Unlike the empty record, the gradual `Any` carries an explicit `kind`
    // tag so it does not decode back as an empty record.
    assert_eq!(json, r#"{"kind":"any","fields":[]}"#);

    let decoded: ValidSchema = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.kind(), SchemaKind::Any);
    assert_eq!(any, decoded);

    // The empty record and the gradual `Any` serialize differently and must
    // not compare equal after a round-trip.
    let empty = Schema::builder().build().unwrap();
    assert_ne!(decoded, empty);
}
