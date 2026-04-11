use nebula_validator::{foundation::Validate, validators::min_length};
use serde_json::json;

#[test]
fn typed_and_dynamic_validation_are_equivalent_for_string_inputs() {
    let validator = min_length(3);

    let typed_valid = validator.validate("alice");
    let dynamic_valid = validator.validate_any(&json!("alice"));
    assert_eq!(
        typed_valid.is_ok(),
        dynamic_valid.is_ok(),
        "typed and dynamic paths must agree for valid input"
    );

    let typed_invalid = validator.validate("hi");
    let dynamic_invalid = validator.validate_any(&json!("hi"));
    assert_eq!(
        typed_invalid.is_err(),
        dynamic_invalid.is_err(),
        "typed and dynamic paths must agree for invalid input"
    );

    let typed_error = typed_invalid.expect_err("typed path must fail");
    let dynamic_error = dynamic_invalid.expect_err("dynamic path must fail");
    assert_eq!(typed_error.code, dynamic_error.code);
}

#[test]
fn dynamic_bridge_preserves_type_mismatch_contract() {
    let validator = min_length(3);
    let err = validator
        .validate_any(&json!(123))
        .expect_err("numeric JSON value must fail for string validator");

    assert_eq!(err.code.as_ref(), "type_mismatch");
    assert_eq!(err.param("expected"), Some("string"));
    assert_eq!(err.param("actual"), Some("number"));
}
