//! JSON Schema export smoke test (`schemars` feature). Run with:
//! `cargo test -p nebula-schema --features schemars json_schema_smoke`

use nebula_schema::{Field, FieldKey, Schema};
use serde_json::Value;

#[test]
fn valid_schema_json_schema_includes_draft_2020_12_and_typed_property() {
    let key = FieldKey::new("name").expect("key");
    let schema = Schema::builder()
        // Literal-only string so the export keeps a top-level `type: string` (not `anyOf` for
        // expression wrappers).
        .add(Field::string(key).required().no_expression())
        .build()
        .expect("build");

    let exported = schema.json_schema().expect("export");
    let value: Value = serde_json::to_value(&exported).expect("serialize");

    assert_eq!(
        value.get("$schema").and_then(|v| v.as_str()),
        Some("https://json-schema.org/draft/2020-12/schema")
    );
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("object"));
    assert!(
        value
            .pointer("/properties/name")
            .is_some_and(|n| n.get("type") == Some(&Value::String("string".to_owned())))
    );
    assert_eq!(value.get("additionalProperties"), Some(&Value::Bool(false)));
}
