//! Roundtrip serde tests for all AuthScheme types.
//!
//! Verifies that `SecretString` fields serialize their actual value (not
//! `[REDACTED]`) and deserialize back correctly.

use nebula_credential::{
    SecretString,
    scheme::{
        Certificate, ChallengeSecret, ConnectionUri, FederatedAssertion, IdentityPassword,
        InstanceBinding, KeyPair, OAuth2Token, OtpSeed, SecretToken, SharedKey, SigningKey,
    },
};

#[test]
fn secret_token_serde_roundtrip() {
    let token = SecretToken::new(SecretString::new("my-secret-key"));
    let json = serde_json::to_string(&token).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("my-secret-key"));
    let recovered: SecretToken = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.token().expose_secret(), "my-secret-key");
}

#[test]
fn identity_password_serde_roundtrip() {
    let auth = IdentityPassword::new("admin", SecretString::new("p@ssw0rd"));
    let json = serde_json::to_string(&auth).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("p@ssw0rd"));
    let recovered: IdentityPassword = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.identity(), "admin");
    assert_eq!(recovered.password().expose_secret(), "p@ssw0rd");
}

#[test]
fn oauth2_token_serde_roundtrip() {
    let token = OAuth2Token::new(SecretString::new("access-tok-xyz"));
    let json = serde_json::to_string(&token).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("access-tok-xyz"));
    let recovered: OAuth2Token = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.access_token().expose_secret(), "access-tok-xyz");
}

#[test]
fn certificate_serde_roundtrip() {
    let cert = Certificate::new("TEST_CERT_CHAIN", SecretString::new("TEST_PRIVATE_KEY"));
    let json = serde_json::to_string(&cert).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("TEST_CERT_CHAIN"));
    assert!(json.contains("TEST_PRIVATE_KEY"));
    let recovered: Certificate = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.cert_chain(), "TEST_CERT_CHAIN");
    assert_eq!(recovered.private_key().expose_secret(), "TEST_PRIVATE_KEY");
}

#[test]
fn key_pair_serde_roundtrip() {
    let kp = KeyPair::new("ssh-rsa AAAA...", SecretString::new("-----BEGIN RSA-----"));
    let json = serde_json::to_string(&kp).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("-----BEGIN RSA-----"));
    let recovered: KeyPair = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.public_key(), "ssh-rsa AAAA...");
    assert_eq!(
        recovered.private_key().expose_secret(),
        "-----BEGIN RSA-----"
    );
}

#[test]
fn certificate_deserializes_without_passphrase_field() {
    // Regression: Option<SecretString> passphrase must default to None when
    // the JSON omits the field. Without #[serde(default)] the custom
    // deserializer would reject missing fields. See PR #526 / CodeRabbit review.
    let json = r#"{"cert_chain":"TEST_CERT_CHAIN","private_key":"TEST_PRIVATE_KEY"}"#;
    let cert: Certificate = serde_json::from_str(json).unwrap();
    assert_eq!(cert.cert_chain(), "TEST_CERT_CHAIN");
    assert_eq!(cert.private_key().expose_secret(), "TEST_PRIVATE_KEY");
    assert!(
        cert.passphrase().is_none(),
        "missing passphrase must default to None"
    );
}

#[test]
fn key_pair_deserializes_without_passphrase_field() {
    // Regression: same as above but for KeyPair. `algorithm` is already
    // `Option<String>` plain, which serde handles — the previous gap was
    // passphrase's custom Option deserializer.
    let json =
        r#"{"public_key":"ssh-rsa AAAA...","private_key":"-----BEGIN RSA-----","algorithm":null}"#;
    let kp: KeyPair = serde_json::from_str(json).unwrap();
    assert_eq!(kp.public_key(), "ssh-rsa AAAA...");
    assert_eq!(kp.private_key().expose_secret(), "-----BEGIN RSA-----");
    assert!(
        kp.passphrase().is_none(),
        "missing passphrase must default to None"
    );
}

#[test]
fn signing_key_serde_roundtrip() {
    let sk = SigningKey::new(SecretString::new("signing-key"), "hmac-sha256");
    let json = serde_json::to_string(&sk).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("signing-key"));
    let recovered: SigningKey = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.key().expose_secret(), "signing-key");
    assert_eq!(recovered.algorithm(), "hmac-sha256");
}

#[test]
fn shared_key_serde_roundtrip() {
    let sk = SharedKey::new(SecretString::new("preshared-secret"));
    let json = serde_json::to_string(&sk).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("preshared-secret"));
    let recovered: SharedKey = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.key().expose_secret(), "preshared-secret");
}

#[test]
fn connection_uri_serde_roundtrip() {
    let cu = ConnectionUri::new(SecretString::new("postgres://user:pass@localhost/db"));
    let json = serde_json::to_string(&cu).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("postgres://user:pass@localhost/db"));
    let recovered: ConnectionUri = serde_json::from_str(&json).unwrap();
    assert_eq!(
        recovered.uri().expose_secret(),
        "postgres://user:pass@localhost/db"
    );
}

#[test]
fn federated_assertion_serde_roundtrip() {
    let fa = FederatedAssertion::new(
        SecretString::new("PHNhbWw+base64"),
        "https://idp.example.com",
    );
    let json = serde_json::to_string(&fa).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("PHNhbWw+base64"));
    let recovered: FederatedAssertion = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.assertion().expose_secret(), "PHNhbWw+base64");
    assert_eq!(recovered.issuer(), "https://idp.example.com");
}

#[test]
fn challenge_secret_serde_roundtrip() {
    let cs = ChallengeSecret::new("admin", SecretString::new("challenge-pw"), "scram-sha256");
    let json = serde_json::to_string(&cs).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("challenge-pw"));
    let recovered: ChallengeSecret = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.identity(), "admin");
    assert_eq!(recovered.secret().expose_secret(), "challenge-pw");
    assert_eq!(recovered.protocol(), "scram-sha256");
}

#[test]
fn otp_seed_serde_roundtrip() {
    let otp = OtpSeed::new(SecretString::new("JBSWY3DPEHPK3PXP"), "totp", 6);
    let json = serde_json::to_string(&otp).unwrap();
    assert!(
        !json.contains("REDACTED"),
        "json must not contain REDACTED: {json}"
    );
    assert!(json.contains("JBSWY3DPEHPK3PXP"));
    let recovered: OtpSeed = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.seed().expose_secret(), "JBSWY3DPEHPK3PXP");
    assert_eq!(recovered.algorithm(), "totp");
    assert_eq!(recovered.digits(), 6);
}

#[test]
fn instance_binding_serde_roundtrip() {
    let ib = InstanceBinding::new("aws", "arn:aws:iam::123456789012:role/MyRole");
    let json = serde_json::to_string(&ib).unwrap();
    assert!(json.contains("aws"));
    assert!(json.contains("arn:aws:iam"));
    let recovered: InstanceBinding = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.provider(), "aws");
    assert_eq!(
        recovered.role_or_account(),
        "arn:aws:iam::123456789012:role/MyRole"
    );
}
