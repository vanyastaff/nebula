//! Serialization round-trip tests for the v2 Schema / Field types.

use nebula_parameter::{Field, Rule, Schema};

#[test]
fn schema_json_roundtrip() {
    let schema = Schema::new()
        .field(Field::text("name").with_label("Name").required())
        .field(Field::text("token").with_label("Token").required().secret());

    let json = serde_json::to_string(&schema).unwrap();
    let back: Schema = serde_json::from_str(&json).unwrap();

    assert_eq!(back.fields.len(), 2);
    assert!(back.contains("name"));
    assert!(back.contains("token"));
    assert!(back.get_field("token").unwrap().meta().secret);
}

#[test]
fn field_meta_is_flat_in_json() {
    let field = Field::text("host")
        .with_label("Host")
        .with_placeholder("localhost")
        .required();

    let json = serde_json::to_value(&field).unwrap();
    assert_eq!(json["id"], "host");
    assert_eq!(json["label"], "Host");
    assert_eq!(json["placeholder"], "localhost");
    assert_eq!(json["required"], true);
    assert_eq!(json["type"], "text");
}

#[test]
fn field_builder_methods() {
    let field = Field::text("my_field")
        .with_label("My Field")
        .with_description("A test field")
        .with_hint("Enter something")
        .required()
        .secret()
        .with_rule(Rule::MinLength {
            min: 3,
            message: None,
        });

    let meta = field.meta();
    assert_eq!(meta.id, "my_field");
    assert_eq!(meta.label, "My Field");
    assert!(meta.description.is_some());
    assert!(meta.required);
    assert!(meta.secret);
    assert!(!meta.rules.is_empty());
}

#[test]
fn schema_convenience_methods() {
    let schema = Schema::new()
        .field(Field::boolean("enabled").with_label("Enabled"))
        .field(Field::number("count").with_label("Count"));

    assert_eq!(schema.len(), 2);
    assert!(!schema.is_empty());
    assert!(schema.contains("enabled"));
    assert!(!schema.contains("missing"));

    let field = schema.get_field("count").unwrap();
    assert!(matches!(field, Field::Number { .. }));
}

#[test]
fn select_field_roundtrip() {
    let field = Field::select("env").with_label("Environment").required();

    let json = serde_json::to_string(&field).unwrap();
    let back: Field = serde_json::from_str(&json).unwrap();

    assert!(back.meta().required);
    assert_eq!(back.meta().id, "env");
}
