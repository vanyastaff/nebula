use serde_json::{Value, json};

use crate::{FieldKey, Property, Schema, SerdeTagging, ValidSchema};

#[test]
fn exports_basic_object_shape_and_required() {
    let schema = Schema::builder()
        .property(
            Property::string(FieldKey::new("name").expect("static key"))
                .required()
                .min_length(2),
        )
        .property(Property::secret(
            FieldKey::new("password").expect("static key"),
        ))
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    assert_eq!(
        json["$schema"],
        json!("https://json-schema.org/draft/2020-12/schema")
    );
    assert_eq!(json["type"], json!("object"));
    assert_eq!(json["required"], json!(["name"]));
    assert_eq!(
        json["properties"]["name"]["x-nebula-resolved-value-schema"]["type"],
        json!("string")
    );
    assert_eq!(
        json["properties"]["name"]["x-nebula-resolved-value-schema"]["minLength"],
        json!(2)
    );
    assert_eq!(
        json["properties"]["password"]["x-nebula-resolved-value-schema"]["type"],
        json!("string")
    );
    assert_eq!(
        json["properties"]["password"]["x-nebula-resolved-value-schema"]["writeOnly"],
        json!(true)
    );
    assert_eq!(
        json["properties"]["name"]["x-nebula-expression-mode"],
        json!("allowed")
    );
    assert_eq!(
        json["properties"]["name"]["x-nebula-required-mode"],
        json!("always")
    );
}

#[test]
fn exports_mode_as_one_of_branches() {
    let schema = Schema::builder()
        .property(
            Property::mode(FieldKey::new("auth").expect("static key"))
                .variant(
                    "none",
                    "None",
                    Property::notice(FieldKey::new("n").expect("static key")),
                )
                .variant(
                    "token",
                    "Token",
                    Property::secret(FieldKey::new("token").expect("static key")).required(),
                ),
        )
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    let one_of = json["properties"]["auth"]["x-nebula-resolved-value-schema"]["oneOf"]
        .as_array()
        .expect("oneOf array");
    assert_eq!(one_of.len(), 2);
    assert!(
        one_of
            .iter()
            .any(|v| v["properties"]["mode"]["const"] == Value::String("none".to_owned()))
    );
    assert!(
        one_of
            .iter()
            .any(|v| v["properties"]["mode"]["const"] == Value::String("token".to_owned()))
    );
    let token = one_of
        .iter()
        .find(|v| v["properties"]["mode"]["const"] == Value::String("token".to_owned()))
        .expect("token branch exists");
    assert_eq!(token["required"], json!(["mode", "value"]));
}

#[test]
fn mode_json_schema_does_not_export_removed_dynamic_flag() {
    let schema = Schema::builder()
        .property(
            Property::mode(FieldKey::new("auth").expect("static key"))
                .variant(
                    "token",
                    "Token",
                    Property::secret(FieldKey::new("token").expect("static key")),
                )
                .default_variant("token"),
        )
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    assert_eq!(
        json["properties"]["auth"]["x-nebula-mode-default-variant"],
        json!("token")
    );
    assert!(
        json["properties"]["auth"]["x-nebula-mode-allow-dynamic"].is_null(),
        "removed compatibility flag should not be exported"
    );
}

#[test]
fn exports_allowed_expression_mode_with_any_of() {
    let schema = Schema::builder()
        .property(Property::dynamic(
            FieldKey::new("runtime").expect("static key"),
        ))
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    assert!(json["properties"]["runtime"]["anyOf"].is_array());
    assert!(json["properties"]["runtime"]["oneOf"].is_null());
}

#[test]
fn exports_number_rules_and_expression_wrapper_contract() {
    let schema = Schema::builder()
        .property(
            Property::number(FieldKey::new("count").expect("static key"))
                .min(1)
                .max(10)
                .with_rule(nebula_validator::Rule::greater_than(2)),
        )
        .property(
            Property::computed(FieldKey::new("total").expect("static key"))
                .returns(crate::field::ComputedReturn::Number),
        )
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    assert_eq!(
        json["properties"]["count"]["x-nebula-resolved-value-schema"]["type"],
        json!("number")
    );
    assert_eq!(
        json["properties"]["count"]["x-nebula-resolved-value-schema"]["minimum"],
        json!(1)
    );
    assert_eq!(
        json["properties"]["count"]["x-nebula-resolved-value-schema"]["maximum"],
        json!(10)
    );
    assert_eq!(
        json["properties"]["count"]["x-nebula-resolved-value-schema"]["exclusiveMinimum"],
        json!(2)
    );

    // Computed fields are ExpressionMode::Required -> wrapper schema.
    assert_eq!(json["properties"]["total"]["type"], json!("object"));
    assert_eq!(json["properties"]["total"]["required"], json!(["$expr"]));
    assert_eq!(
        json["properties"]["total"]["x-nebula-expression-mode"],
        json!("required")
    );
    assert_eq!(
        json["properties"]["total"]["x-nebula-resolved-value-schema"]["type"],
        json!("number")
    );
}

/// Regression: every property must carry `x-nebula-resolved-value-schema`,
/// regardless of the field's ExpressionMode. Boolean fields default to
/// Forbidden — they used to omit the extension key.
#[test]
fn resolved_value_schema_extension_is_emitted_for_forbidden_mode() {
    let schema = Schema::builder()
        .property(Property::boolean(
            FieldKey::new("flag").expect("static key"),
        ))
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();

    assert_eq!(
        json["properties"]["flag"]["x-nebula-expression-mode"],
        json!("forbidden")
    );
    assert_eq!(
        json["properties"]["flag"]["x-nebula-resolved-value-schema"]["type"],
        json!("boolean"),
        "Forbidden-mode fields must still expose x-nebula-resolved-value-schema"
    );
}

#[test]
fn read_alias_is_an_accepted_property_with_metadata() {
    let schema = Schema::builder()
        .property(
            Property::string(FieldKey::new("internal_id").expect("static key"))
                .read_alias("externalId")
                .expect("valid alias"),
        )
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    // Canonical and alias properties both carry the field's constraints.
    assert!(json["properties"]["internal_id"].is_object());
    assert!(
        json["properties"]["externalId"].is_object(),
        "read-alias must be an accepted input property"
    );
    assert_eq!(
        json["properties"]["internal_id"]["x-nebula-read-aliases"],
        json!(["externalId"])
    );
    assert_eq!(json["additionalProperties"], json!(true));
}

#[test]
fn emit_as_is_metadata_only_input_property_stays_canonical() {
    let schema = Schema::builder()
        .property(
            Property::string(FieldKey::new("internal_id").expect("static key"))
                .emit_as("externalId")
                .expect("valid emit_as key"),
        )
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    // The input property stays canonical — emit_as is output-only.
    assert!(json["properties"]["internal_id"].is_object());
    assert!(
        json["properties"].get("externalId").is_none(),
        "emit_as must not become an input property"
    );
    // The projected output key is exposed as metadata for output validators.
    assert_eq!(
        json["properties"]["internal_id"]["x-nebula-emit-as"],
        json!("externalId")
    );
}

#[test]
fn required_field_with_read_alias_uses_any_of_not_flat_required() {
    let schema = Schema::builder()
        .property(
            Property::string(FieldKey::new("email").expect("static key"))
                .required()
                .read_alias("emailAddress")
                .expect("valid alias"),
        )
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    // A flat `required: [email]` would reject an alias-only submission that
    // `validate` accepts, so the constraint is an anyOf of required clauses.
    assert!(
        json.get("required").is_none(),
        "an alias-bearing required field must not be a flat required entry"
    );
    let all_of = json["allOf"].as_array().expect("allOf array");
    let any_of = all_of[0]["anyOf"].as_array().expect("anyOf array");
    let required_keys: Vec<&str> = any_of
        .iter()
        .map(|clause| clause["required"][0].as_str().expect("required key string"))
        .collect();
    assert!(required_keys.contains(&"email"));
    assert!(required_keys.contains(&"emailAddress"));
}

#[test]
fn exports_external_union_as_oneof_with_unit_string_const() {
    let schema = ValidSchema::union(
        Property::mode(FieldKey::new("auth").expect("static key"))
            .variant(
                "oauth",
                "OAuth",
                Property::object(FieldKey::new("oauth").expect("static key")).property(
                    Property::secret(FieldKey::new("token").expect("static key")).required(),
                ),
            )
            .variant_empty("none", "None"),
        SerdeTagging::External,
    )
    .expect("union builds");

    let json = schema.json_schema().expect("json schema export").to_value();
    let one_of = json["oneOf"].as_array().expect("oneOf array");
    assert_eq!(one_of.len(), 2);

    // External data variant: { "oauth": <payload> }, required [oauth].
    let oauth = one_of
        .iter()
        .find(|b| b["properties"]["oauth"].is_object())
        .expect("oauth data branch");
    assert_eq!(oauth["required"], json!(["oauth"]));
    assert_eq!(oauth["additionalProperties"], json!(false));

    // External unit variant: the bare string const "none" (C1: serde emits a
    // bare string for a unit variant, not { "none": {} }).
    assert!(
        one_of.iter().any(|b| b["const"] == json!("none")),
        "external unit variant must export as a bare string const"
    );
    assert!(
        json.get("discriminator").is_none(),
        "external tagging has no discriminator"
    );
}

#[test]
fn exports_adjacent_union_with_discriminator_and_omitted_unit_content() {
    let schema = ValidSchema::union(
        Property::mode(FieldKey::new("event").expect("static key"))
            .variant(
                "click",
                "Click",
                Property::object(FieldKey::new("click").expect("static key"))
                    .property(Property::number(FieldKey::new("x").expect("static key")).required()),
            )
            .variant_empty("noop", "No-op"),
        SerdeTagging::Adjacent {
            tag: "type".to_owned(),
            content: "data".to_owned(),
        },
    )
    .expect("union builds");

    let json = schema.json_schema().expect("json schema export").to_value();
    assert_eq!(json["discriminator"]["propertyName"], json!("type"));
    let one_of = json["oneOf"].as_array().expect("oneOf array");

    // Adjacent data variant: tag const + content, required [type, data].
    let click = one_of
        .iter()
        .find(|b| b["properties"]["type"]["const"] == json!("click"))
        .expect("click branch");
    assert!(click["properties"]["data"].is_object());
    assert_eq!(click["required"], json!(["type", "data"]));

    // Adjacent unit variant: tag const only, content omitted, required [type].
    let noop = one_of
        .iter()
        .find(|b| b["properties"]["type"]["const"] == json!("noop"))
        .expect("noop branch");
    assert!(
        noop["properties"].get("data").is_none(),
        "adjacent unit variant must omit the content key"
    );
    assert_eq!(noop["required"], json!(["type"]));
}

#[test]
fn required_field_without_alias_stays_flat_required() {
    let schema = Schema::builder()
        .property(Property::string(FieldKey::new("name").expect("static key")).required())
        .build()
        .expect("valid schema");

    let json = schema.json_schema().expect("json schema export").to_value();
    assert_eq!(json["required"], json!(["name"]));
    assert!(json.get("allOf").is_none());
}
