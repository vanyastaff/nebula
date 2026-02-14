use nebula_credential::core::{CredentialId, ValidationError};
use nebula_credential::utils::SecretString;

#[test]
fn test_credential_id_valid() {
    // Valid IDs - alphanumeric, hyphens, underscores
    assert!(CredentialId::new("github_token").is_ok());
    assert!(CredentialId::new("aws-access-key-123").is_ok());
    assert!(CredentialId::new("db_password_prod").is_ok());
    assert!(CredentialId::new("API_KEY_2024").is_ok());
    assert!(CredentialId::new("service-account-1").is_ok());
    assert!(CredentialId::new("a").is_ok()); // Single character
    assert!(CredentialId::new("123").is_ok()); // Numbers only
    assert!(CredentialId::new("a-b_c-d_e").is_ok()); // Mixed separators
}

#[test]
fn test_credential_id_empty() {
    // Empty string should fail
    let result = CredentialId::new("");
    assert!(result.is_err());

    match result {
        Err(ValidationError::EmptyCredentialId) => {
            // Expected error
        }
        _ => panic!("Expected ValidationError::EmptyCredentialId"),
    }
}

#[test]
fn test_credential_id_invalid_chars() {
    // Invalid characters should fail
    let invalid_ids = vec![
        "../etc/passwd",            // Path traversal
        "token with spaces",        // Spaces
        "token/with/slashes",       // Slashes
        "token\\with\\backslashes", // Backslashes
        "token.with.dots",          // Dots
        "token@with@ats",           // At signs
        "token#with#hashes",        // Hashes
        "token$with$dollars",       // Dollar signs
        "token%with%percents",      // Percents
        "token!with!exclamations",  // Exclamations
        "token:with:colons",        // Colons
        "token;with;semicolons",    // Semicolons
        "token,with,commas",        // Commas
        "token[with]brackets",      // Brackets
        "token{with}braces",        // Braces
        "token(with)parens",        // Parentheses
        "token<with>angles",        // Angle brackets
        "token|with|pipes",         // Pipes
        "token&with&ampersands",    // Ampersands
    ];

    for id_str in invalid_ids {
        let result = CredentialId::new(id_str);
        assert!(result.is_err(), "Expected '{}' to be invalid", id_str);

        match result {
            Err(ValidationError::InvalidCredentialId { id, reason }) => {
                assert_eq!(id, id_str);
                assert!(reason.contains("invalid characters"));
            }
            _ => panic!(
                "Expected ValidationError::InvalidCredentialId for '{}'",
                id_str
            ),
        }
    }
}

#[test]
fn test_secret_string_redacted() {
    let secret = SecretString::new("my-super-secret-password-12345");

    // Debug should show [REDACTED]
    let debug_str = format!("{:?}", secret);
    assert_eq!(debug_str, "[REDACTED]");

    // Display should show [REDACTED]
    let display_str = format!("{}", secret);
    assert_eq!(display_str, "[REDACTED]");

    // Should not contain the actual secret
    assert!(!debug_str.contains("my-super-secret"));
    assert!(!display_str.contains("my-super-secret"));
}

#[test]
fn test_secret_string_expose_secret() {
    let secret = SecretString::new("test-secret-value");

    // expose_secret should allow access within closure
    let length = secret.expose_secret(|s| s.len());
    assert_eq!(length, 17);

    // Can perform operations on the secret
    let uppercase = secret.expose_secret(|s| s.to_uppercase());
    assert_eq!(uppercase, "TEST-SECRET-VALUE");

    // Can check content
    let contains_test = secret.expose_secret(|s| s.contains("test"));
    assert!(contains_test);

    // Can get substring
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

#[test]
fn test_credential_id_display() {
    let id = CredentialId::new("my_credential_123").unwrap();
    let display_str = format!("{}", id);
    assert_eq!(display_str, "my_credential_123");
}

#[test]
fn test_credential_id_as_str() {
    let id = CredentialId::new("test_id").unwrap();
    let s: &str = id.as_str();
    assert_eq!(s, "test_id");
}

#[test]
fn test_credential_id_into_string() {
    let id = CredentialId::new("convert_test").unwrap();
    let s: String = id.into();
    assert_eq!(s, "convert_test");
}

#[test]
fn test_credential_id_try_from_string() {
    use std::convert::TryFrom;

    // Valid conversion
    let result = CredentialId::try_from("valid_id".to_string());
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_str(), "valid_id");

    // Invalid conversion
    let result = CredentialId::try_from("invalid id".to_string());
    assert!(result.is_err());
}
