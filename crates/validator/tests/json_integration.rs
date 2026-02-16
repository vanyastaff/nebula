//! Integration tests for serde_json::Value validation.

#![cfg(feature = "serde")]

use nebula_validator::combinators::{json_field, json_field_optional};
use nebula_validator::foundation::{Validate, ValidateExt};
use nebula_validator::validators::min_length;
use serde_json::{Value, json};

// ============================================================================
// EXISTING TESTS
// ============================================================================

#[test]
fn validate_string_value_directly() {
    let validator = min_length(3);
    assert!(validator.validate_any(&json!("hello")).is_ok());
    assert!(validator.validate_any(&json!("hi")).is_err());
}

#[test]
fn validate_config_structure() {
    let data = json!({
        "server": {
            "host": "localhost",
            "port": 8080
        },
        "database": {
            "url": "postgres://localhost/db"
        }
    });

    let host = json_field("/server/host", min_length(1));
    let db_url = json_field("/database/url", min_length(5));
    let combined = host.and(db_url);
    assert!(combined.validate(&data).is_ok());
}

#[test]
fn validate_array_element_by_index() {
    let data = json!({
        "servers": [
            {"host": "web1", "port": 80},
            {"host": "web2", "port": 443}
        ]
    });

    let first_host = json_field("/servers/0/host", min_length(1));
    assert!(first_host.validate(&data).is_ok());

    let second_host = json_field("/servers/1/host", min_length(1));
    assert!(second_host.validate(&data).is_ok());
}

#[test]
fn optional_field_missing() {
    let data = json!({"name": "Alice"});
    let optional = json_field_optional("/email", min_length(5));
    assert!(optional.validate(&data).is_ok());
}

#[test]
fn type_mismatch_gives_clear_error() {
    let validator = min_length(1);
    let err = validator.validate_any(&json!(42)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
    assert!(err.message.contains("string"));
}

#[test]
fn null_value_type_mismatch() {
    let validator = min_length(1);
    let err = validator.validate_any(&json!(null)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
}

#[test]
fn min_size_with_json_array() {
    use nebula_validator::validators::min_size;

    let validator = min_size::<Value>(2);
    assert!(validator.validate_any(&json!([1, 2, 3])).is_ok());
    assert!(validator.validate_any(&json!([1])).is_err());
}

#[test]
fn composed_json_field_validators() {
    let data = json!({
        "user": {
            "name": "Alice",
            "age": 28
        },
        "settings": {
            "theme": "dark"
        }
    });

    let v =
        json_field("/user/name", min_length(1)).and(json_field("/settings/theme", min_length(1)));

    assert!(v.validate(&data).is_ok());
}

// ============================================================================
// A: NUMERIC VALIDATORS
// ============================================================================

#[test]
fn numeric_min_value_i64() {
    use nebula_validator::validators::min;

    let v = min::<i64>(18);
    assert!(v.validate_any(&json!(25)).is_ok());
    assert!(v.validate_any(&json!(18)).is_ok());

    let err = v.validate_any(&json!(10)).unwrap_err();
    assert_eq!(err.code.as_ref(), "min");
}

#[test]
fn numeric_max_value_i64() {
    use nebula_validator::validators::max;

    let v = max::<i64>(100);
    assert!(v.validate_any(&json!(50)).is_ok());
    assert!(v.validate_any(&json!(100)).is_ok());
    assert!(v.validate_any(&json!(101)).is_err());
}

#[test]
fn numeric_in_range_i64() {
    use nebula_validator::validators::in_range;

    let v = in_range::<i64>(1, 65535);
    assert!(v.validate_any(&json!(8080)).is_ok());
    assert!(v.validate_any(&json!(1)).is_ok());
    assert!(v.validate_any(&json!(0)).is_err());
    assert!(v.validate_any(&json!(65536)).is_err());
}

#[test]
fn numeric_f64_validation() {
    use nebula_validator::validators::{in_range, min};

    let v = min::<f64>(0.0);
    assert!(v.validate_any(&json!(3.14)).is_ok());
    assert!(v.validate_any(&json!(-0.5)).is_err());

    // integers widen to f64
    let v2 = in_range::<f64>(0.0, 100.0);
    assert!(v2.validate_any(&json!(42)).is_ok());
}

#[test]
fn numeric_positive_i64() {
    use nebula_validator::validators::positive;

    let v = positive::<i64>();
    assert!(v.validate_any(&json!(42)).is_ok());
    assert!(v.validate_any(&json!(1)).is_ok());

    let err = v.validate_any(&json!(-1)).unwrap_err();
    assert_eq!(err.code.as_ref(), "positive");
}

#[test]
fn numeric_json_field_port() {
    use nebula_validator::validators::in_range;

    let data = json!({"server": {"port": 8080}});
    let v = json_field("/server/port", in_range::<i64>(1, 65535));
    assert!(v.validate(&data).is_ok());

    let bad = json!({"server": {"port": 0}});
    let err = v.validate(&bad).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/server/port"));
}

// ============================================================================
// B: STRING FORMAT VALIDATORS
// ============================================================================

#[test]
fn string_email_json() {
    use nebula_validator::validators::email;

    let v = email();
    assert!(v.validate_any(&json!("user@example.com")).is_ok());
    assert!(v.validate_any(&json!("not-an-email")).is_err());
}

#[test]
fn string_url_json() {
    use nebula_validator::validators::url;

    let v = url();
    assert!(v.validate_any(&json!("https://example.com")).is_ok());
    assert!(v.validate_any(&json!("not a url")).is_err());
}

#[test]
fn string_regex_json() {
    use nebula_validator::validators::matches_regex;

    let v = matches_regex("^[a-z0-9_]+$").unwrap();
    assert!(v.validate_any(&json!("hello_world")).is_ok());
    assert!(v.validate_any(&json!("Hello World!")).is_err());
}

#[test]
fn string_contains_json() {
    use nebula_validator::validators::contains;

    let v = contains("hello");
    assert!(v.validate_any(&json!("say hello world")).is_ok());
    assert!(v.validate_any(&json!("goodbye")).is_err());
}

#[test]
fn string_uuid_json_field() {
    use nebula_validator::validators::Uuid;

    let data = json!({"id": "550e8400-e29b-41d4-a716-446655440000"});
    let v = json_field("/id", Uuid::default());
    assert!(v.validate(&data).is_ok());

    let bad = json!({"id": "not-a-uuid"});
    assert!(v.validate(&bad).is_err());
}

// ============================================================================
// C: BOOLEAN VALIDATORS
// ============================================================================

#[test]
fn bool_is_true_json() {
    use nebula_validator::validators::is_true;

    let v = is_true();
    assert!(v.validate_any(&json!(true)).is_ok());
    assert!(v.validate_any(&json!(false)).is_err());
}

#[test]
fn bool_type_mismatch() {
    use nebula_validator::validators::is_true;

    // string "true" is not bool true
    let err = is_true().validate_any(&json!("true")).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
    assert!(err.message.contains("boolean"));
}

// ============================================================================
// D: COLLECTION VALIDATORS
// ============================================================================

#[test]
fn collection_max_size_json() {
    use nebula_validator::validators::max_size;

    let v = max_size::<Value>(3);
    assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    assert!(v.validate_any(&json!([1, 2, 3, 4])).is_err());
}

#[test]
fn collection_exact_size_json() {
    use nebula_validator::validators::exact_size;

    let v = exact_size::<Value>(2);
    assert!(v.validate_any(&json!([1, 2])).is_ok());
    assert!(v.validate_any(&json!([1])).is_err());
    assert!(v.validate_any(&json!([1, 2, 3])).is_err());
}

#[test]
fn collection_not_empty_json() {
    use nebula_validator::validators::not_empty_collection;

    let v = not_empty_collection::<Value>();
    assert!(v.validate_any(&json!([1])).is_ok());

    let err = v.validate_any(&json!([])).unwrap_err();
    assert_eq!(err.code.as_ref(), "not_empty");
}

#[test]
fn collection_size_range_json() {
    use nebula_validator::validators::size_range;

    let v = size_range::<Value>(1, 5);
    assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    assert!(v.validate_any(&json!([])).is_err());
    assert!(v.validate_any(&json!([1, 2, 3, 4, 5, 6])).is_err());
}

// ============================================================================
// E: COMBINATORS WITH JSON
// ============================================================================

#[test]
fn or_combinator_json() {
    // Field can be either a non-empty string OR a positive number
    use nebula_validator::validators::positive;

    let v = json_field("/value", min_length(1)).or(json_field("/value", positive::<i64>()));

    assert!(v.validate(&json!({"value": "hello"})).is_ok());
    assert!(v.validate(&json!({"value": 42})).is_ok());
    // Neither string nor positive number
    assert!(v.validate(&json!({"value": -1})).is_err());
}

#[test]
fn not_combinator_json() {
    use nebula_validator::combinators::not;
    use nebula_validator::validators::contains;

    // Status must NOT contain "error"
    let v = not(json_field("/status", contains("error")));
    assert!(v.validate(&json!({"status": "ok"})).is_ok());
    assert!(v.validate(&json!({"status": "fatal error"})).is_err());
}

#[test]
fn when_combinator_json() {
    use nebula_validator::combinators::when;
    use nebula_validator::validators::email;

    // Validate email only when notify=true
    let v = when(json_field("/email", email()), |v: &Value| {
        v.get("notify").and_then(|n| n.as_bool()).unwrap_or(false)
    });

    // notify=true, invalid email → fail
    let err = v
        .validate(&json!({"notify": true, "email": "bad"}))
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/email"));

    // notify=false, invalid email → pass (skipped)
    assert!(
        v.validate(&json!({"notify": false, "email": "bad"}))
            .is_ok()
    );

    // notify=true, valid email → pass
    assert!(
        v.validate(&json!({"notify": true, "email": "user@example.com"}))
            .is_ok()
    );
}

#[test]
fn unless_combinator_json() {
    use nebula_validator::combinators::unless;

    // Require bio with 10+ chars, UNLESS account type is "bot"
    let v = unless(json_field("/bio", min_length(10)), |v: &Value| {
        v.get("type").and_then(|t| t.as_str()) == Some("bot")
    });

    // Human with short bio → fail
    assert!(v.validate(&json!({"type": "human", "bio": "hi"})).is_err());

    // Bot with short bio → pass (skipped)
    assert!(v.validate(&json!({"type": "bot", "bio": "hi"})).is_ok());

    // Human with long bio → pass
    assert!(
        v.validate(&json!({"type": "human", "bio": "A detailed biography here"}))
            .is_ok()
    );
}

#[test]
fn with_message_json() {
    use nebula_validator::combinators::with_message;

    let v = with_message(json_field("/name", min_length(1)), "Name is required");
    let err = v.validate(&json!({"name": ""})).unwrap_err();
    assert_eq!(err.message.as_ref(), "Name is required");
}

#[test]
fn with_code_json() {
    use nebula_validator::combinators::with_code;

    let v = with_code(json_field("/name", min_length(1)), "MISSING_NAME");
    let err = v.validate(&json!({"name": ""})).unwrap_err();
    assert_eq!(err.code.as_ref(), "MISSING_NAME");
}

// ============================================================================
// F: EDGE CASES
// ============================================================================

#[test]
fn deeply_nested_4_levels() {
    use nebula_validator::validators::min;

    let data = json!({
        "a": {
            "b": {
                "c": {
                    "d": 42
                }
            }
        }
    });

    let v = json_field("/a/b/c/d", min::<i64>(1));
    assert!(v.validate(&data).is_ok());
}

#[test]
fn empty_object_missing_fields() {
    let data = json!({});
    let v = json_field("/name", min_length(1));
    let err = v.validate(&data).unwrap_err();
    assert_eq!(err.code.as_ref(), "path_not_found");
    assert_eq!(err.field.as_deref(), Some("/name"));
}

#[test]
fn empty_array_not_empty() {
    use nebula_validator::validators::not_empty_collection;

    let v = json_field("/items", not_empty_collection::<Value>());
    let err = v.validate(&json!({"items": []})).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/items"));
}

#[test]
fn multiple_field_errors() {
    use nebula_validator::validators::email;

    let data = json!({
        "name": "",
        "email": "not-email"
    });

    // Validate each field independently and collect errors
    let validators: Vec<Box<dyn Validate<Input = Value>>> = vec![
        Box::new(json_field("/name", min_length(1))),
        Box::new(json_field("/email", email())),
    ];

    let errors: Vec<_> = validators
        .iter()
        .filter_map(|v| v.validate(&data).err())
        .collect();

    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].field.as_deref(), Some("/name"));
    assert_eq!(errors[1].field.as_deref(), Some("/email"));
}

// ============================================================================
// G: REAL-WORLD SCENARIOS
// ============================================================================

#[test]
fn user_registration_payload() {
    use nebula_validator::validators::in_range;
    use nebula_validator::validators::is_true;
    use nebula_validator::validators::{email, max_length};

    let validator = json_field("/name", min_length(1))
        .and(json_field("/name", max_length(100)))
        .and(json_field("/email", email()))
        .and(json_field("/password", min_length(8)))
        .and(json_field("/age", in_range::<i64>(13, 120)))
        .and(json_field("/terms_accepted", is_true()));

    // Valid registration
    let valid = json!({
        "name": "Alice",
        "email": "alice@example.com",
        "password": "securepass123",
        "age": 28,
        "terms_accepted": true
    });
    assert!(validator.validate(&valid).is_ok());

    // Missing name
    let no_name = json!({
        "name": "",
        "email": "alice@example.com",
        "password": "securepass123",
        "age": 28,
        "terms_accepted": true
    });
    let err = validator.validate(&no_name).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/name"));

    // Too young
    let too_young = json!({
        "name": "Bob",
        "email": "bob@example.com",
        "password": "securepass123",
        "age": 10,
        "terms_accepted": true
    });
    let err = validator.validate(&too_young).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/age"));

    // Terms not accepted
    let no_terms = json!({
        "name": "Charlie",
        "email": "charlie@example.com",
        "password": "securepass123",
        "age": 30,
        "terms_accepted": false
    });
    let err = validator.validate(&no_terms).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/terms_accepted"));
}

#[test]
fn server_config_payload() {
    use nebula_validator::validators::contains;
    use nebula_validator::validators::{in_range, positive};

    let validator = json_field("/host", min_length(1))
        .and(json_field("/port", in_range::<i64>(1, 65535)))
        .and(json_field("/workers", positive::<i64>()))
        .and(json_field_optional("/tls/cert_path", min_length(1)))
        .and(json_field_optional(
            "/log_level",
            contains("info").or(contains("warn")).or(contains("error")),
        ));

    // Valid config
    let valid = json!({
        "host": "0.0.0.0",
        "port": 8080,
        "workers": 4,
        "tls": {
            "cert_path": "/etc/ssl/cert.pem"
        },
        "log_level": "info"
    });
    assert!(validator.validate(&valid).is_ok());

    // Valid config without optional fields
    let minimal = json!({
        "host": "localhost",
        "port": 3000,
        "workers": 1
    });
    assert!(validator.validate(&minimal).is_ok());

    // Invalid port
    let bad_port = json!({
        "host": "localhost",
        "port": 0,
        "workers": 1
    });
    let err = validator.validate(&bad_port).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/port"));
}
