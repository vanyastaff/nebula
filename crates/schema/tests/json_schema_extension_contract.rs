//! Evidence for the boundary between exported annotations and runtime validation.

#![cfg(feature = "schemars")]

use nebula_schema::{
    AuthoredValue, Field, Predicate, Rule, ScalarSchema, Schema, SerdeTagging, ValidSchema,
    field_key, schema_of,
};
use serde_json::json;

#[test]
fn export_version_identifies_every_root_shape() {
    let external = ValidSchema::union(
        Field::mode(field_key!("choice")).variant_empty("none", "None"),
        SerdeTagging::External,
    )
    .unwrap();
    let adjacent = ValidSchema::union(
        Field::mode(field_key!("choice")).variant_empty("none", "None"),
        SerdeTagging::Adjacent {
            tag: "type".to_owned(),
            content: "data".to_owned(),
        },
    )
    .unwrap();
    for (root, schema) in [
        ("record", ValidSchema::empty()),
        ("scalar", ValidSchema::scalar(ScalarSchema::null()).unwrap()),
        ("any", ValidSchema::any()),
        ("external union", external),
        ("adjacent union", adjacent),
    ] {
        let exported = schema.json_schema().unwrap().to_value();
        assert_eq!(
            exported["x-nebula-schema-version"].as_u64(),
            Some(2),
            "{root} export must identify the definition/export writer contract"
        );
    }
}

#[test]
fn generic_validation_does_not_enforce_export_version() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).required().no_expression())
        .build()
        .unwrap();
    let mut exported = schema.json_schema().unwrap().to_value();
    exported["x-nebula-schema-version"] = json!(999);
    let validator = jsonschema::validator_for(&exported).unwrap();
    assert!(validator.is_valid(&json!({"name": "structurally valid"})));
    assert!(!validator.is_valid(&json!({"name": 42})));
    assert!(!validator.is_valid(&json!({})));
}

#[derive(Schema, serde::Serialize)]
struct DisplayHints {
    #[property(
        display(
            label = "number",
            description = "{\"type\":\"boolean\",\"minimum\":0}",
            placeholder = "false",
            hint = "text",
            group = "schema",
            widget = text
        ),
        input(required, expressions = forbidden),
        validate(length(min = 3, max = 8))
    )]
    value: String,
}

#[test]
fn display_attributes_cannot_override_validation_keywords() {
    let schema = schema_of::<DisplayHints>().expect("display hints describe a checked string");
    let exported = schema
        .json_schema()
        .expect("display schema exports")
        .to_value();
    let property = &exported["properties"]["value"];
    assert_eq!(property["title"], "number");
    assert_eq!(
        property["description"],
        "{\"type\":\"boolean\",\"minimum\":0}"
    );
    assert_eq!(property["type"], "string");
    assert_eq!(property["minLength"], 3);
    assert_eq!(property["maxLength"], 8);
    assert_eq!(exported["required"], json!(["value"]));

    let validator = jsonschema::validator_for(&exported).expect("export is valid JSON Schema");
    let input = serde_json::to_value(DisplayHints {
        value: "hello".to_owned(),
    })
    .unwrap();
    assert!(validator.is_valid(&input));
    let resolved = schema
        .validate(AuthoredValue::from_data(input.clone()).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(resolved.values().to_json(), input);
    for invalid in [
        json!({}),
        json!({"value": false}),
        json!({"value": "hi"}),
        json!({"value": "too long a value"}),
    ] {
        assert!(
            !validator.is_valid(&invalid),
            "display hints must not admit {invalid}"
        );
        assert!(
            schema
                .validate(AuthoredValue::from_data(invalid).unwrap())
                .is_err()
        );
    }
}

#[test]
fn file_hints_preserve_opaque_string_reference_validation() {
    let schema = Schema::builder()
        .add(
            Field::file(field_key!("upload"))
                .required()
                .no_expression()
                .accept("image/png")
                .max_size(0),
        )
        .add(
            Field::file(field_key!("attachments"))
                .required()
                .no_expression()
                .multiple()
                .accept("application/pdf")
                .max_size(1),
        )
        .build()
        .unwrap();
    let exported = schema.json_schema().unwrap().to_value();
    assert_eq!(exported["properties"]["upload"]["type"], "string");
    assert_eq!(
        exported["properties"]["upload"]["x-nebula-file-accept"],
        "image/png"
    );
    assert_eq!(
        exported["properties"]["upload"]["x-nebula-file-max-size"],
        0
    );
    assert_eq!(
        exported["properties"]["attachments"]["items"],
        json!({"type": "string"})
    );

    // No referenced file is opened: these strings provide neither bytes nor MIME evidence.
    let references = json!({
        "upload": "opaque:not-an-existing-file.exe",
        "attachments": ["opaque:text-document.txt", "opaque:another-reference"]
    });
    let validator = jsonschema::validator_for(&exported).unwrap();
    assert!(validator.is_valid(&references));
    let resolved = schema
        .validate(AuthoredValue::from_data(references.clone()).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(resolved.values().to_json(), references);

    for invalid in [
        json!({"upload": {"bytes": [1, 2], "mime": "image/png"}, "attachments": ["reference"]}),
        json!({"upload": ["reference"], "attachments": ["reference"]}),
        json!({"upload": "reference", "attachments": [42]}),
    ] {
        assert!(
            !validator.is_valid(&invalid),
            "file references retain their declared shape"
        );
        assert!(
            schema
                .validate(AuthoredValue::from_data(invalid).unwrap())
                .is_err()
        );
    }
}

#[test]
fn unknown_extension_keywords_do_not_authorize_generic_validation() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).required().no_expression())
        .build()
        .unwrap();
    let mut exported = schema.json_schema().unwrap().to_value();
    exported["x-nebula-future-policy"] = json!({"version": 999, "must_reject": true});
    let validator = jsonschema::validator_for(&exported).unwrap();

    assert!(validator.is_valid(&json!({"name": "accepted by structural keywords"})));
    assert!(!validator.is_valid(&json!({"name": 42})));
    assert!(!validator.is_valid(&json!({})));
}

#[test]
fn generic_validation_does_not_enforce_exported_root_rules() {
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("enabled")))
        .root_rule(Rule::predicate(Predicate::eq("/enabled", json!(true)).unwrap()).unwrap())
        .build()
        .unwrap();
    let exported = schema.json_schema().unwrap().to_value();
    let validator = jsonschema::validator_for(&exported).unwrap();
    let rejected_by_runtime = json!({"enabled": false});
    assert!(validator.is_valid(&rejected_by_runtime));
    assert!(
        schema
            .validate(AuthoredValue::from_data(rejected_by_runtime).unwrap())
            .is_err(),
        "a root-rule annotation is not a generic validator assertion"
    );

    let accepted = json!({"enabled": true});
    assert!(validator.is_valid(&accepted));
    let resolved = schema
        .validate(AuthoredValue::from_data(accepted.clone()).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(resolved.values().to_json(), accepted);
}
