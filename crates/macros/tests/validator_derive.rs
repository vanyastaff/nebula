//! Integration tests for #[derive(Validator)] format attributes.

use nebula_macros::Validator;
use nebula_validator::foundation::Validate;

#[derive(Validator, Clone)]
#[validator(message = "email config invalid")]
struct EmailConfig {
    #[validate(email)]
    sender: String,

    #[validate(url)]
    webhook: String,
}

#[derive(Validator, Clone)]
struct NetworkConfig {
    #[validate(ipv4)]
    host: String,

    #[validate(hostname)]
    fqdn: String,

    #[validate(uuid)]
    id: String,
}

#[derive(Validator, Clone)]
struct DateConfig {
    #[validate(date)]
    start: String,

    #[validate(date_time)]
    created_at: String,

    #[validate(time)]
    daily_at: String,
}

#[derive(Validator, Clone)]
struct RegexConfig {
    #[validate(regex = r"^\d{4}$")]
    code: String,

    #[validate(regex = r"^[a-z]+$")]
    slug: String,
}

#[derive(Validator, Clone)]
struct OptionalConfig {
    #[validate(email)]
    reply_to: Option<String>,

    #[validate(ipv4)]
    override_ip: Option<String>,
}

#[test]
fn email_valid() {
    let c = EmailConfig {
        sender: "user@example.com".into(),
        webhook: "https://example.com/hook".into(),
    };
    assert!(c.validate_fields().is_ok());
}

#[test]
fn email_invalid() {
    let c = EmailConfig {
        sender: "not-an-email".into(),
        webhook: "https://example.com/hook".into(),
    };
    assert!(c.validate_fields().is_err());
}

#[test]
fn url_invalid() {
    let c = EmailConfig {
        sender: "user@example.com".into(),
        webhook: "not-a-url".into(),
    };
    assert!(c.validate_fields().is_err());
}

#[test]
fn network_valid() {
    let c = NetworkConfig {
        host: "192.168.0.1".into(),
        fqdn: "sub.example.com".into(),
        id: "550e8400-e29b-41d4-a716-446655440000".into(),
    };
    assert!(c.validate_fields().is_ok());
}

#[test]
fn ipv4_invalid() {
    let c = NetworkConfig {
        host: "999.0.0.1".into(),
        fqdn: "ok.com".into(),
        id: "550e8400-e29b-41d4-a716-446655440000".into(),
    };
    assert!(c.validate_fields().is_err());
}

#[test]
fn uuid_invalid() {
    let c = NetworkConfig {
        host: "192.168.0.1".into(),
        fqdn: "ok.com".into(),
        id: "not-a-uuid".into(),
    };
    assert!(c.validate_fields().is_err());
}

#[test]
fn date_valid() {
    let c = DateConfig {
        start: "2024-01-15".into(),
        created_at: "2024-01-15T10:30:00Z".into(),
        daily_at: "08:00:00".into(),
    };
    assert!(c.validate_fields().is_ok());
}

#[test]
fn date_invalid() {
    let c = DateConfig {
        start: "not-a-date".into(),
        created_at: "2024-01-15T10:30:00Z".into(),
        daily_at: "08:00:00".into(),
    };
    assert!(c.validate_fields().is_err());
}

#[test]
fn regex_valid() {
    let c = RegexConfig {
        code: "1234".into(),
        slug: "hello".into(),
    };
    assert!(c.validate_fields().is_ok());
}

#[test]
fn regex_invalid_code() {
    let c = RegexConfig {
        code: "abc".into(),
        slug: "hello".into(),
    };
    assert!(c.validate_fields().is_err());
}

#[test]
fn optional_none_skipped() {
    let c = OptionalConfig {
        reply_to: None,
        override_ip: None,
    };
    assert!(c.validate_fields().is_ok());
}

#[test]
fn optional_some_invalid() {
    let c = OptionalConfig {
        reply_to: Some("not-an-email".into()),
        override_ip: None,
    };
    assert!(c.validate_fields().is_err());
}

#[test]
fn validate_trait_impl_works() {
    let c = EmailConfig {
        sender: "user@example.com".into(),
        webhook: "https://example.com".into(),
    };
    // Validate trait delegates to validate_fields
    assert!(c.validate(&c).is_ok());

    let bad = EmailConfig {
        sender: "bad".into(),
        webhook: "https://example.com".into(),
    };
    assert!(bad.validate(&bad).is_err());
}
