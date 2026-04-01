//! Roundtrip serde tests for all AuthScheme types.
//!
//! Verifies that `SecretString` fields serialize their actual value (not
//! `[REDACTED]`) and deserialize back correctly.

use nebula_core::SecretString;
use nebula_credential::scheme::{
    ApiKeyAuth, AwsAuth, BasicAuth, BearerToken, CertificateAuth, DatabaseAuth, HeaderAuth,
    HmacSecret, KerberosAuth, LdapAuth, OAuth2Token, SamlAuth, SshAuth,
};

#[test]
fn bearer_token_serde_roundtrip() {
    let token = BearerToken::new(SecretString::new("my-secret-key"));
    let json = serde_json::to_string(&token).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("my-secret-key"));
    let recovered: BearerToken = serde_json::from_str(&json).unwrap();
    recovered
        .expose()
        .expose_secret(|s| assert_eq!(s, "my-secret-key"));
}

#[test]
fn basic_auth_serde_roundtrip() {
    let auth = BasicAuth::new("admin", SecretString::new("p@ssw0rd"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("p@ssw0rd"));
    let recovered: BasicAuth = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.username, "admin");
    recovered
        .password()
        .expose_secret(|s| assert_eq!(s, "p@ssw0rd"));
}

#[test]
fn database_auth_serde_roundtrip() {
    let auth = DatabaseAuth::new("localhost", 5432, "mydb", "user", SecretString::new("db-pass"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("db-pass"));
    let recovered: DatabaseAuth = serde_json::from_str(&json).unwrap();
    recovered
        .password()
        .expose_secret(|s| assert_eq!(s, "db-pass"));
}

#[test]
fn api_key_auth_serde_roundtrip() {
    let auth = ApiKeyAuth::header("X-API-Key", SecretString::new("key-123"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("key-123"));
    let recovered: ApiKeyAuth = serde_json::from_str(&json).unwrap();
    recovered
        .key()
        .expose_secret(|s| assert_eq!(s, "key-123"));
}

#[test]
fn hmac_secret_serde_roundtrip() {
    let hmac = HmacSecret::new(SecretString::new("signing-key"), "sha256");
    let json = serde_json::to_string(&hmac).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("signing-key"));
    let recovered: HmacSecret = serde_json::from_str(&json).unwrap();
    recovered
        .secret()
        .expose_secret(|s| assert_eq!(s, "signing-key"));
}

#[test]
fn aws_auth_serde_roundtrip() {
    let auth = AwsAuth::new(
        SecretString::new("AKIAIOSFODNN7EXAMPLE"),
        SecretString::new("wJalrXUtnFEMI"),
        "us-east-1",
    )
    .with_session_token(SecretString::new("session-tok"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(json.contains("wJalrXUtnFEMI"));
    assert!(json.contains("session-tok"));
    let recovered: AwsAuth = serde_json::from_str(&json).unwrap();
    recovered
        .access_key_id()
        .expose_secret(|s| assert_eq!(s, "AKIAIOSFODNN7EXAMPLE"));
    recovered
        .secret_access_key()
        .expose_secret(|s| assert_eq!(s, "wJalrXUtnFEMI"));
    recovered
        .session_token()
        .unwrap()
        .expose_secret(|s| assert_eq!(s, "session-tok"));
}

#[test]
fn aws_auth_serde_roundtrip_no_session_token() {
    let auth = AwsAuth::new(
        SecretString::new("AKID"),
        SecretString::new("SECRET"),
        "eu-west-1",
    );
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"));
    let recovered: AwsAuth = serde_json::from_str(&json).unwrap();
    assert!(recovered.session_token().is_none());
}

#[test]
fn ssh_auth_password_serde_roundtrip() {
    let auth = SshAuth::with_password("host.example.com", 22, "root", SecretString::new("ssh-pw"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("ssh-pw"));
    let _recovered: SshAuth = serde_json::from_str(&json).unwrap();
}

#[test]
fn ssh_auth_keypair_serde_roundtrip() {
    let auth = SshAuth::with_key_pair(
        "host.example.com",
        22,
        "root",
        SecretString::new("-----BEGIN RSA-----"),
        Some(SecretString::new("my-passphrase")),
    );
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("-----BEGIN RSA-----"));
    assert!(json.contains("my-passphrase"));
    let _recovered: SshAuth = serde_json::from_str(&json).unwrap();
}

#[test]
fn ssh_auth_keypair_no_passphrase_serde_roundtrip() {
    let auth = SshAuth::with_key_pair(
        "host.example.com",
        22,
        "root",
        SecretString::new("-----BEGIN RSA-----"),
        None,
    );
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"));
    let _recovered: SshAuth = serde_json::from_str(&json).unwrap();
}

#[test]
fn certificate_auth_serde_roundtrip() {
    let auth = CertificateAuth::new(
        SecretString::new("-----BEGIN CERTIFICATE-----"),
        SecretString::new("-----BEGIN PRIVATE KEY-----"),
    );
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("-----BEGIN CERTIFICATE-----"));
    assert!(json.contains("-----BEGIN PRIVATE KEY-----"));
    let recovered: CertificateAuth = serde_json::from_str(&json).unwrap();
    recovered
        .cert_pem()
        .expose_secret(|s| assert_eq!(s, "-----BEGIN CERTIFICATE-----"));
    recovered
        .key_pem()
        .expose_secret(|s| assert_eq!(s, "-----BEGIN PRIVATE KEY-----"));
}

#[test]
fn oauth2_token_serde_roundtrip() {
    let token = OAuth2Token::new(SecretString::new("access-tok-xyz"));
    let json = serde_json::to_string(&token).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("access-tok-xyz"));
    let recovered: OAuth2Token = serde_json::from_str(&json).unwrap();
    recovered
        .access_token()
        .expose_secret(|s| assert_eq!(s, "access-tok-xyz"));
}

#[test]
fn header_auth_serde_roundtrip() {
    let auth = HeaderAuth::new("X-Api-Key", SecretString::new("header-secret"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("header-secret"));
    let recovered: HeaderAuth = serde_json::from_str(&json).unwrap();
    recovered
        .value()
        .expose_secret(|s| assert_eq!(s, "header-secret"));
}

#[test]
fn kerberos_auth_serde_roundtrip() {
    let expiry = chrono::Utc::now() + chrono::Duration::hours(8);
    let auth = KerberosAuth::new(
        "user@REALM.COM",
        "REALM.COM",
        SecretString::new("ticket-data"),
        expiry,
    );
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("ticket-data"));
    let recovered: KerberosAuth = serde_json::from_str(&json).unwrap();
    recovered
        .service_ticket()
        .expose_secret(|s| assert_eq!(s, "ticket-data"));
}

#[test]
fn ldap_auth_simple_serde_roundtrip() {
    let auth = LdapAuth::simple(
        "ldap.example.com",
        389,
        "cn=admin,dc=example,dc=com",
        SecretString::new("ldap-pass"),
    );
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("ldap-pass"));
    let _recovered: LdapAuth = serde_json::from_str(&json).unwrap();
}

#[test]
fn saml_auth_with_assertion_serde_roundtrip() {
    let auth =
        SamlAuth::new("user@example.com").with_assertion(SecretString::new("PHNhbWw+base64"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"), "json must not contain REDACTED: {json}");
    assert!(json.contains("PHNhbWw+base64"));
    let recovered: SamlAuth = serde_json::from_str(&json).unwrap();
    recovered
        .assertion_b64()
        .unwrap()
        .expose_secret(|s| assert_eq!(s, "PHNhbWw+base64"));
}

#[test]
fn saml_auth_without_assertion_serde_roundtrip() {
    let auth = SamlAuth::new("user@example.com");
    let json = serde_json::to_string(&auth).unwrap();
    assert!(!json.contains("REDACTED"));
    let recovered: SamlAuth = serde_json::from_str(&json).unwrap();
    assert!(recovered.assertion_b64().is_none());
}
