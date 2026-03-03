use nebula_credential::core::{CredentialId, CredentialLabel, ValidationError};
use nebula_credential::utils::SecretString;

#[test]
fn test_credential_label_valid() {
    // Valid labels - alphanumeric, hyphens, underscores
    assert!(CredentialLabel::new("github_token").is_ok());
    assert!(CredentialLabel::new("aws-access-key-123").is_ok());
    assert!(CredentialLabel::new("db_password_prod").is_ok());
    assert!(CredentialLabel::new("API_KEY_2024").is_ok());
    assert!(CredentialLabel::new("service-account-1").is_ok());
    assert!(CredentialLabel::new("a").is_ok());
    assert!(CredentialLabel::new("123").is_ok());
    assert!(CredentialLabel::new("a-b_c-d_e").is_ok());
}

#[test]
fn test_credential_label_empty() {
    let result = CredentialLabel::new("");
    assert!(result.is_err());

    match result {
        Err(ValidationError::EmptyCredentialId) => {}
        _ => panic!("Expected ValidationError::EmptyCredentialId"),
    }
}

#[test]
fn test_credential_label_invalid_chars() {
    let invalid_ids = vec![
        "../etc/passwd",
        "token with spaces",
        "token/with/slashes",
        "token\\with\\backslashes",
        "token.with.dots",
        "token@with@ats",
    ];

    for id_str in invalid_ids {
        let result = CredentialLabel::new(id_str);
        assert!(result.is_err(), "Expected '{}' to be invalid", id_str);

        match result {
            Err(ValidationError::InvalidCredentialId { id, reason }) => {
                assert_eq!(id, id_str);
                assert!(reason.contains("invalid"));
            }
            _ => panic!(
                "Expected ValidationError::InvalidCredentialId for '{}'",
                id_str
            ),
        }
    }
}

#[test]
fn test_credential_id_parse_valid_uuid() {
    let id = CredentialId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
    assert_eq!(id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn test_credential_id_parse_invalid() {
    assert!(CredentialId::parse("not-a-uuid").is_err());
    assert!(CredentialId::parse("github_token").is_err());
}

#[test]
fn test_credential_id_display() {
    let id = CredentialId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
    assert_eq!(format!("{}", id), "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn test_secret_string_redacted() {
    let secret = SecretString::new("my-super-secret-password-12345");

    let debug_str = format!("{:?}", secret);
    assert_eq!(debug_str, "[REDACTED]");

    let display_str = format!("{}", secret);
    assert_eq!(display_str, "[REDACTED]");

    assert!(!debug_str.contains("my-super-secret"));
    assert!(!display_str.contains("my-super-secret"));
}

#[test]
fn test_secret_string_expose_secret() {
    let secret = SecretString::new("test-secret-value");

    let length = secret.expose_secret(|s| s.len());
    assert_eq!(length, 17);

    let uppercase = secret.expose_secret(|s| s.to_uppercase());
    assert_eq!(uppercase, "TEST-SECRET-VALUE");

    let contains_test = secret.expose_secret(|s| s.contains("test"));
    assert!(contains_test);

    let first_four = secret.expose_secret(|s| s.chars().take(4).collect::<String>());
    assert_eq!(first_four, "test");
}

#[test]
fn test_secret_string_len_is_empty() {
    let secret1 = SecretString::new("password");
    assert_eq!(secret1.len(), 8);
    assert!(!secret1.is_empty());

    let secret2 = SecretString::new("");
    assert_eq!(secret2.len(), 0);
    assert!(secret2.is_empty());
}
