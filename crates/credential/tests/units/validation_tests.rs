use nebula_core::CredentialKey;
use nebula_credential::{CredentialId, SecretString};

#[test]
fn test_credential_key_valid() {
    // CredentialKey from nebula-core - validated domain key format
    assert!(CredentialKey::new("github_token").is_ok());
    assert!(CredentialKey::new("aws_access_key").is_ok());
    assert!(CredentialKey::new("db_password_prod").is_ok());
    assert!(CredentialKey::new("api_key").is_ok());
    assert!(CredentialKey::new("oauth2_github").is_ok());
}

#[test]
fn test_credential_key_invalid() {
    assert!(CredentialKey::new("").is_err());
    // domain_key allows hyphens and digits — only truly invalid chars fail
    assert!(CredentialKey::new("has spaces").is_err());
    assert!(CredentialKey::new("special@chars").is_err());
}

#[test]
fn test_credential_id_parse_roundtrip() {
    let id = CredentialId::new();
    let s = id.to_string();
    let parsed: CredentialId = s.parse().unwrap();
    assert_eq!(id, parsed);
}

#[test]
fn test_credential_id_parse_invalid() {
    assert!("not-a-ulid".parse::<CredentialId>().is_err());
    assert!("github_token".parse::<CredentialId>().is_err());
}

#[test]
fn test_credential_id_rejects_wrong_prefix() {
    // Take a valid CredentialId, swap its prefix to "exe_", and verify rejection.
    let id = CredentialId::new();
    let id_str = id.to_string();
    let ulid_part = id_str
        .strip_prefix("cred_")
        .expect("CredentialId must start with cred_");
    let wrong_prefix = format!("exe_{ulid_part}");
    assert!(
        wrong_prefix.parse::<CredentialId>().is_err(),
        "a valid ULID with wrong prefix must be rejected"
    );
}

#[test]
fn test_credential_id_display() {
    let id = CredentialId::new();
    let display = format!("{id}");
    assert!(
        display.starts_with("cred_"),
        "expected cred_ prefix, got: {display}"
    );
}

#[test]
fn test_secret_string_redacted() {
    let secret = SecretString::new("my-super-secret-password-12345");

    let debug_str = format!("{secret:?}");
    assert_eq!(debug_str, "[REDACTED]");

    let display_str = format!("{secret}");
    assert_eq!(display_str, "[REDACTED]");

    assert!(!debug_str.contains("my-super-secret"));
    assert!(!display_str.contains("my-super-secret"));
}

#[test]
fn test_secret_string_expose_secret() {
    let secret = SecretString::new("test-secret-value");

    let length = secret.expose_secret().len();
    assert_eq!(length, 17);

    let uppercase = secret.expose_secret().to_uppercase();
    assert_eq!(uppercase, "TEST-SECRET-VALUE");

    let contains_test = secret.expose_secret().contains("test");
    assert!(contains_test);

    let first_four: String = secret.expose_secret().chars().take(4).collect();
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
