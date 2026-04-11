use nebula_validator::foundation::ValidationError;

use super::helpers::assert_no_secrets;

#[test]
fn sensitive_param_values_are_redacted_in_errors() {
    let error = ValidationError::new("auth_failed", "Credential rejected")
        .with_param("password", "super-secret")
        .with_param("token", "api-token-123")
        .with_param("username", "alice");

    let serialized = error.to_json_value().to_string();
    assert_no_secrets(&serialized);
    assert!(serialized.contains("\"username\":\"alice\""));
}

#[test]
fn display_output_does_not_expose_forbidden_tokens() {
    let error = ValidationError::new("auth_failed", "Authentication failed")
        .with_param("bearer_token", "bearer_verysecret")
        .with_help("Rotate credentials and retry");
    let rendered = error.to_string();
    assert_no_secrets(&rendered);
}
