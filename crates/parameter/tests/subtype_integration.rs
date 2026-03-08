//! Integration tests for subtype system.

use nebula_parameter::prelude::*;

#[test]
fn text_subtype_serialization() {
    let email = TextParameter::email("user_email", "User Email")
        .description("Contact email address")
        .required();

    let json = serde_json::to_value(&email).unwrap();
    assert_eq!(json["subtype"], "email");
    // required is a boolean field in metadata
    assert_eq!(email.metadata.required, true);

    // Deserialize back
    let deserialized: TextParameter = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized.subtype, TextSubtype::Email);
}

#[test]
fn text_subtype_auto_validation() {
    let email = TextParameter::email("email", "Email");

    // Should have pattern validation auto-applied
    assert!(!email.validation.is_empty());
    assert!(
        email
            .options
            .as_ref()
            .and_then(|o| o.pattern.as_ref())
            .is_some()
    );

    // Verify pattern contains @
    assert!(
        email
            .options
            .as_ref()
            .unwrap()
            .pattern
            .as_ref()
            .unwrap()
            .contains("@")
    );
}

#[test]
fn text_subtype_auto_sensitive() {
    let password = TextParameter::password("password", "Password");

    // Should be marked as sensitive
    assert!(password.metadata.sensitive);
}

#[test]
fn number_subtype_serialization() {
    let port = NumberParameter::port("server_port", "Server Port")
        .description("HTTP server port")
        .required();

    let json = serde_json::to_value(&port).unwrap();
    assert_eq!(json["subtype"], "port");
    assert_eq!(port.metadata.required, true);

    // Deserialize back
    let deserialized: NumberParameter = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized.subtype, NumberSubtype::Port);
}

#[test]
fn number_subtype_auto_constraints() {
    let percentage = NumberParameter::percentage("opacity", "Opacity");

    // Should have min/max constraints auto-applied
    let opts = percentage.options.as_ref().unwrap();
    assert_eq!(opts.min, Some(0.0));
    assert_eq!(opts.max, Some(100.0));

    assert!(!percentage.validation.is_empty());
}

#[test]
fn number_port_constraints() {
    let port = NumberParameter::port("port", "Port");

    // Should have port range (1-65535)
    let opts = port.options.as_ref().unwrap();
    assert_eq!(opts.min, Some(1.0));
    assert_eq!(opts.max, Some(65535.0));
    assert_eq!(opts.precision, Some(0));
}

#[test]
fn custom_subtype_with_builder() {
    let url = TextParameter::new("api_url", "API URL")
        .subtype(TextSubtype::Url)
        .description("Base URL for API")
        .placeholder("https://api.example.com")
        .required();

    assert_eq!(url.subtype, TextSubtype::Url);
    assert!(
        url.options
            .as_ref()
            .and_then(|o| o.pattern.as_ref())
            .is_some()
    );
}

#[test]
fn text_subtype_default_is_plain() {
    let text = TextParameter::new("text", "Text");
    assert_eq!(text.subtype, TextSubtype::Plain);

    // Plain subtype should not be serialized (default value)
    let json = serde_json::to_value(&text).unwrap();
    assert!(!json.as_object().unwrap().contains_key("subtype"));
}

#[test]
fn number_subtype_default_is_none() {
    let number = NumberParameter::new("number", "Number");
    assert_eq!(number.subtype, NumberSubtype::None);

    // None subtype should not be serialized (default value)
    let json = serde_json::to_value(&number).unwrap();
    assert!(!json.as_object().unwrap().contains_key("subtype"));
}

#[test]
fn text_subtype_json_type() {
    let json_param = TextParameter::new("config", "Config").subtype(TextSubtype::Json);

    assert!(json_param.subtype.is_code());
    assert_eq!(json_param.subtype.description(), "JSON string");
}

#[test]
fn number_subtype_timestamp() {
    let timestamp =
        NumberParameter::new("created_at", "Created At").subtype(NumberSubtype::Timestamp);

    // Check description
    assert_eq!(timestamp.subtype.description(), "Unix timestamp (seconds)");
}

#[test]
fn subtype_roundtrip_in_parameter_def() {
    let email = ParameterDef::Text(TextParameter::email("email", "Email").required());

    let json = serde_json::to_value(&email).unwrap();
    let deserialized: ParameterDef = serde_json::from_value(json).unwrap();

    if let ParameterDef::Text(text) = deserialized {
        assert_eq!(text.subtype, TextSubtype::Email);
        assert!(text.metadata.required);
    } else {
        panic!("Expected Text parameter");
    }
}

#[test]
fn subtype_in_collection() {
    let collection = ParameterCollection::new()
        .with(ParameterDef::Text(
            TextParameter::email("email", "Email").required(),
        ))
        .with(ParameterDef::Number(
            NumberParameter::port("port", "Port").default_value(8080.0),
        ));

    // Serialize the entire collection
    let json = serde_json::to_value(&collection).unwrap();
    let params = json["parameters"].as_array().unwrap();

    assert_eq!(params[0]["subtype"], "email");
    assert_eq!(params[1]["subtype"], "port");

    // Deserialize back
    let deserialized: ParameterCollection = serde_json::from_value(json).unwrap();
    // Collection has 2 parameters
    let json_again = serde_json::to_value(&deserialized).unwrap();
    assert_eq!(json_again["parameters"].as_array().unwrap().len(), 2);
}

#[test]
fn text_subtype_file_path() {
    let file = TextParameter::new("config_file", "Config File").subtype(TextSubtype::FilePath);

    assert_eq!(file.subtype, TextSubtype::FilePath);
    assert!(!file.subtype.is_sensitive());
}

#[test]
fn number_subtype_byte_size() {
    let size = NumberParameter::new("max_size", "Max Size").subtype(NumberSubtype::ByteSize);

    assert_eq!(size.subtype, NumberSubtype::ByteSize);
    assert_eq!(size.subtype.description(), "Byte size");
}
