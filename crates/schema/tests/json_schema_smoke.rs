//! JSON Schema export smoke test (`schemars` feature). Run with:
//! `cargo test -p nebula-schema --features schemars json_schema_smoke`

use nebula_schema::{Field, FieldKey, Schema};
use serde_json::{Value, json};

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

#[test]
fn json_schema_extension_snapshot() {
    let schema = Schema::builder()
        .add(Field::boolean(FieldKey::new("flag").expect("key")))
        .add(
            Field::mode(FieldKey::new("auth").expect("key"))
                .variant_empty("none", "None")
                .default_variant("none"),
        )
        .build()
        .expect("build");

    let value = schema.json_schema().expect("export").to_value();
    let auth = &value["properties"]["auth"];
    let flag = &value["properties"]["flag"];
    let snapshot = json!({
        "auth": {
            "default_variant": auth["x-nebula-mode-default-variant"],
            "expression_mode": auth["x-nebula-expression-mode"],
            "kind": auth["x-nebula-field-kind"],
            "required_mode": auth["x-nebula-required-mode"],
            "resolved_one_of_len": auth["x-nebula-resolved-value-schema"]["oneOf"]
                .as_array()
                .map(Vec::len),
            "visibility_mode": auth["x-nebula-visibility-mode"],
        },
        "flag": {
            "expression_mode": flag["x-nebula-expression-mode"],
            "kind": flag["x-nebula-field-kind"],
            "required_mode": flag["x-nebula-required-mode"],
            "resolved": flag["x-nebula-resolved-value-schema"],
            "visibility_mode": flag["x-nebula-visibility-mode"],
        },
    });

    insta::assert_json_snapshot!(snapshot, @r###"
    {
      "auth": {
        "default_variant": "none",
        "expression_mode": "allowed",
        "kind": "mode",
        "required_mode": "never",
        "resolved_one_of_len": 1,
        "visibility_mode": "always"
      },
      "flag": {
        "expression_mode": "forbidden",
        "kind": "boolean",
        "required_mode": "never",
        "resolved": {
          "type": "boolean"
        },
        "visibility_mode": "always"
      }
    }
    "###);
}
