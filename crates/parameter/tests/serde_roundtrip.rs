//! Serialization round-trip tests for the v3 Parameter / ParameterCollection types.

use nebula_parameter::{Parameter, ParameterCollection, ParameterType, Rule};

#[test]
fn collection_json_roundtrip() {
    let collection = ParameterCollection::new()
        .add(Parameter::string("name").label("Name").required())
        .add(
            Parameter::string("token")
                .label("Token")
                .required()
                .secret(),
        );

    let json = serde_json::to_string(&collection).unwrap();
    let back: ParameterCollection = serde_json::from_str(&json).unwrap();

    assert_eq!(back.parameters.len(), 2);
    assert!(back.contains("name"));
    assert!(back.contains("token"));
    assert!(back.get("token").unwrap().secret);
}

#[test]
fn parameter_fields_are_flat_in_json() {
    let param = Parameter::string("host")
        .label("Host")
        .placeholder("localhost")
        .required();

    let json = serde_json::to_value(&param).unwrap();
    assert_eq!(json["id"], "host");
    assert_eq!(json["label"], "Host");
    assert_eq!(json["placeholder"], "localhost");
    assert_eq!(json["required"], true);
    assert_eq!(json["type"], "string");
}

#[test]
fn parameter_builder_methods() {
    let param = Parameter::string("my_field")
        .label("My Field")
        .description("A test field")
        .hint("Enter something")
        .required()
        .secret()
        .with_rule(Rule::MinLength {
            min: 3,
            message: None,
        });

    assert_eq!(param.id, "my_field");
    assert_eq!(param.label.as_deref(), Some("My Field"));
    assert!(param.description.is_some());
    assert!(param.required);
    assert!(param.secret);
    assert!(!param.rules.is_empty());
}

#[test]
fn collection_convenience_methods() {
    let collection = ParameterCollection::new()
        .add(Parameter::boolean("enabled").label("Enabled"))
        .add(Parameter::number("count").label("Count"));

    assert_eq!(collection.len(), 2);
    assert!(!collection.is_empty());
    assert!(collection.contains("enabled"));
    assert!(!collection.contains("missing"));

    let param = collection.get("count").unwrap();
    assert!(matches!(
        param.param_type,
        ParameterType::Number { integer: false, .. }
    ));
}

#[test]
fn select_parameter_roundtrip() {
    let param = Parameter::select("env").label("Environment").required();

    let json = serde_json::to_string(&param).unwrap();
    let back: Parameter = serde_json::from_str(&json).unwrap();

    assert!(back.required);
    assert_eq!(back.id, "env");
}
