//! Tests for improved DX features.

use nebula_parameter::{param_def, param_values, prelude::*};

#[test]
fn builder_pattern_with_chaining() {
    let param = TextParameter::new("email", "Email Address")
        .required()
        .min_length(5)
        .max_length(255)
        .pattern(r"^.+@.+\..+$")
        .placeholder("user@example.com")
        .description("Your email address");

    assert_eq!(param.key(), "email");
    assert_eq!(param.name(), "Email Address");
    assert!(param.is_required());
    assert!(!param.is_sensitive());
    assert!(param.options.is_some());
    assert_eq!(param.validation.len(), 3); // min_length, max_length, pattern
}

#[test]
fn number_parameter_builder() {
    let param = NumberParameter::new("port", "Port Number")
        .default_value(8080.0)
        .range(1000.0, 65535.0)
        .step(1.0)
        .precision(0);

    assert_eq!(param.default, Some(8080.0));
    assert!(param.options.is_some());

    let opts = param.options.unwrap();
    assert_eq!(opts.min, Some(1000.0));
    assert_eq!(opts.max, Some(65535.0));
    assert_eq!(opts.step, Some(1.0));
    assert_eq!(opts.precision, Some(0));

    assert_eq!(param.validation.len(), 2); // min and max
}

#[test]
fn param_values_macro() {
    let values = param_values! {
        "name" => "Alice",
        "age" => 30,
        "active" => true,
    };

    assert_eq!(values.len(), 3);
    assert_eq!(values.get_string("name"), Some("Alice"));
    assert_eq!(values.get_f64("age"), Some(30.0));
    assert_eq!(values.get_bool("active"), Some(true));
}

#[test]
fn param_def_macro_text() {
    let def = param_def!(text "username", "Username", required, sensitive);

    assert_eq!(def.key(), "username");
    assert_eq!(def.name(), "Username");
    assert!(def.is_required());
    assert!(def.is_sensitive());
}

#[test]
fn param_def_macro_number() {
    let def = param_def!(number "timeout", "Timeout", default = 30.0);

    assert_eq!(def.key(), "timeout");
    if let ParameterDef::Number(num) = def {
        assert_eq!(num.default, Some(30.0));
    } else {
        panic!("Expected Number variant");
    }
}

#[test]
fn parameter_type_trait_methods() {
    let mut param = TextParameter::new("test", "Test");

    // Trait methods work
    assert_eq!(param.key(), "test");
    assert_eq!(param.name(), "Test");
    assert!(!param.is_required());
    assert!(!param.is_sensitive());

    // Builder methods from trait
    param = param
        .required()
        .sensitive()
        .description("Test description")
        .placeholder("Enter test")
        .hint("This is a hint");

    assert!(param.is_required());
    assert!(param.is_sensitive());
    assert_eq!(
        param.metadata.description.as_deref(),
        Some("Test description")
    );
    assert_eq!(param.metadata.placeholder.as_deref(), Some("Enter test"));
    assert_eq!(param.metadata.hint.as_deref(), Some("This is a hint"));
}

#[test]
fn values_get_as_typed() {
    use serde_json::json;

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Config {
        host: String,
        port: u16,
    }

    let values = param_values! {
        "config" => json!({
            "host": "localhost",
            "port": 8080
        }),
    };

    let config: Config = values
        .get_as("config")
        .expect("key exists")
        .expect("deserialization succeeds");

    assert_eq!(config.host, "localhost");
    assert_eq!(config.port, 8080);
}

#[test]
fn values_set_json() {
    #[derive(serde::Serialize)]
    struct MyData {
        value: i32,
    }

    let mut values = ParameterValues::new();
    values.set_json("data", MyData { value: 42 }).unwrap();

    assert_eq!(values.get("data").unwrap()["value"], 42);
}

#[test]
fn values_merge() {
    let mut values1 = param_values! {
        "a" => 1,
        "b" => 2,
    };

    let values2 = param_values! {
        "b" => 20,
        "c" => 3,
    };

    values1.merge(&values2);

    assert_eq!(values1.get_f64("a"), Some(1.0));
    assert_eq!(values1.get_f64("b"), Some(20.0)); // overwritten
    assert_eq!(values1.get_f64("c"), Some(3.0));
}

#[test]
fn values_additional_accessors() {
    use serde_json::json;

    let values = param_values! {
        "int" => 42,
        "arr" => json!([1, 2, 3]),
        "obj" => json!({"key": "value"}),
    };

    assert_eq!(values.get_i64("int"), Some(42));
    assert_eq!(values.get_array("arr").map(|a| a.len()), Some(3));
    assert!(values.get_object("obj").is_some());
}

#[test]
fn collection_with_improved_params() {
    let collection = ParameterCollection::new()
        .with(param_def!(text "name", "Name", required))
        .with(param_def!(number "age", "Age", default = 25.0))
        .with(param_def!(checkbox "active", "Active"));

    assert_eq!(collection.len(), 3);

    let values = param_values! {
        "name" => "Bob",
        "age" => 30,
        "active" => true,
    };

    assert!(collection.validate(&values).is_ok());
}

#[test]
fn validation_with_builder_added_rules() {
    let param = TextParameter::new("username", "Username")
        .required()
        .min_length(3)
        .max_length(20);

    let collection = ParameterCollection::new().with(ParameterDef::Text(param));

    // Valid
    let values1 = param_values! { "username" => "alice" };
    assert!(collection.validate(&values1).is_ok());

    // Too short
    let values2 = param_values! { "username" => "ab" };
    let errors = collection.validate(&values2).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code(), "PARAM_VALIDATION");

    // Too long
    let values3 = param_values! { "username" => "a".repeat(21) };
    let errors = collection.validate(&values3).unwrap_err();
    assert_eq!(errors.len(), 1);
}
