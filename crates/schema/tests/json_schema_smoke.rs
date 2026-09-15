//! JSON Schema export smoke test (`schemars` feature). Run with:
//! `cargo test -p nebula-schema --features schemars json_schema_smoke`

use nebula_schema::{Field, FieldKey, Schema, SelectOption};
use serde_json::{Value, json};
use std::collections::BTreeSet;

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
    assert_eq!(value.get("additionalProperties"), Some(&Value::Bool(true)));
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

#[test]
fn json_schema_x_nebula_extension_set_is_frozen() {
    let schema = Schema::builder()
        .add(
            Field::string(FieldKey::new("name").expect("key"))
                .read_alias("legacy_name")
                .expect("alias")
                .emit_as("display_name")
                .expect("emit key"),
        )
        .add(
            Field::file(FieldKey::new("avatar").expect("key"))
                .accept("image/png")
                .max_size(1_048_576),
        )
        .add(
            Field::select(FieldKey::new("region").expect("key"))
                .dynamic()
                .multiple()
                .allow_custom(),
        )
        .add(
            Field::select(FieldKey::new("provider").expect("key")).extend_options([
                SelectOption::new(json!("github"), "GitHub"),
                SelectOption::new(json!("legacy"), "Legacy").disabled(),
            ]),
        )
        .add(
            Field::mode(FieldKey::new("auth").expect("key"))
                .variant_empty("none", "None")
                .default_variant("none"),
        )
        .root_rule(nebula_schema::Rule::custom("engine.check").expect("rule"))
        .build()
        .expect("build");

    let value = schema.json_schema().expect("export").to_value();
    let mut extensions = BTreeSet::new();
    collect_extensions(&value, &mut extensions);
    insta::assert_json_snapshot!(extensions, @r###"
    [
      "x-nebula-disabled",
      "x-nebula-emit-as",
      "x-nebula-expression-mode",
      "x-nebula-field-kind",
      "x-nebula-file-accept",
      "x-nebula-file-max-size",
      "x-nebula-mode-default-variant",
      "x-nebula-read-aliases",
      "x-nebula-required-mode",
      "x-nebula-resolved-value-schema",
      "x-nebula-root-rules",
      "x-nebula-schema-version",
      "x-nebula-select-allow-custom",
      "x-nebula-select-dynamic",
      "x-nebula-select-multiple",
      "x-nebula-visibility-mode"
    ]
    "###);
}

fn collect_extensions(value: &Value, extensions: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key.starts_with("x-nebula-") {
                    extensions.insert(key.clone());
                }
                collect_extensions(value, extensions);
            }
        },
        Value::Array(values) => {
            for value in values {
                collect_extensions(value, extensions);
            }
        },
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {},
    }
}
