//! Integration tests for #[derive(Validator)] format attributes.
//!
//! This file tests complex scenarios including:
//! - Multiple validators on single fields
//! - Real-world domain structs
//! - Numeric validators (min/max)
//! - Edge cases and error accumulation
//! - Optional fields with multiple validators

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

// ============================================================================
// MULTIPLE VALIDATORS ON SINGLE FIELD
// ============================================================================

#[derive(Validator, Clone)]
struct PasswordPolicy {
    #[validate(min_length = 8, max_length = 128)]
    password: String,
}

#[test]
fn multiple_length_validators_valid() {
    let p = PasswordPolicy {
        password: "secure_password_123".into(),
    };
    assert!(p.validate_fields().is_ok());
}

#[test]
fn multiple_length_validators_too_short() {
    let p = PasswordPolicy {
        password: "short".into(),
    };
    assert!(p.validate_fields().is_err());
}

#[test]
fn multiple_length_validators_too_long() {
    let p = PasswordPolicy {
        password: "a".repeat(200),
    };
    assert!(p.validate_fields().is_err());
}

#[derive(Validator, Clone)]
struct SecureEmail {
    #[validate(email, min_length = 5, max_length = 254)]
    address: String,
}

#[test]
fn length_plus_format_valid() {
    let e = SecureEmail {
        address: "user@example.com".into(),
    };
    assert!(e.validate_fields().is_ok());
}

#[test]
fn length_plus_format_invalid_email() {
    let e = SecureEmail {
        address: "not-an-email".into(),
    };
    assert!(e.validate_fields().is_err());
}

#[test]
fn length_plus_format_too_short() {
    let e = SecureEmail {
        address: "a@b".into(), // valid email format but too short
    };
    assert!(e.validate_fields().is_err());
}

#[derive(Validator, Clone)]
struct ProductCode {
    #[validate(regex = r"^[A-Z]{2}\d{4}$", min_length = 6, max_length = 6)]
    code: String,
}

#[test]
fn regex_plus_length_valid() {
    let p = ProductCode {
        code: "AB1234".into(),
    };
    assert!(p.validate_fields().is_ok());
}

#[test]
fn regex_plus_length_invalid_pattern() {
    let p = ProductCode {
        code: "ab1234".into(), // lowercase fails regex
    };
    assert!(p.validate_fields().is_err());
}

#[test]
fn regex_plus_length_wrong_length() {
    let p = ProductCode {
        code: "AB12".into(), // too short
    };
    assert!(p.validate_fields().is_err());
}

// ============================================================================
// REAL-WORLD COMPLEX STRUCTS
// ============================================================================

#[derive(Validator, Clone)]
#[validator(message = "user registration failed")]
struct UserRegistration {
    #[validate(min_length = 3, max_length = 20, regex = r"^[a-zA-Z0-9_]+$")]
    username: String,

    #[validate(email, max_length = 254)]
    email: String,

    #[validate(min_length = 8)]
    password: String,

    #[validate(min = 13, max = 120)]
    age: u32,

    #[validate(url)]
    website: Option<String>,
}

#[test]
fn user_registration_all_valid() {
    let user = UserRegistration {
        username: "alice_123".into(),
        email: "alice@example.com".into(),
        password: "secretpass".into(),
        age: 25,
        website: Some("https://alice.dev".into()),
    };
    assert!(user.validate_fields().is_ok());
}

#[test]
fn user_registration_optional_none_valid() {
    let user = UserRegistration {
        username: "bob".into(),
        email: "bob@test.com".into(),
        password: "password123".into(),
        age: 30,
        website: None,
    };
    assert!(user.validate_fields().is_ok());
}

#[test]
fn user_registration_username_too_short() {
    let user = UserRegistration {
        username: "ab".into(),
        email: "user@example.com".into(),
        password: "password123".into(),
        age: 25,
        website: None,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn user_registration_username_invalid_chars() {
    let user = UserRegistration {
        username: "user@name".into(), // @ not allowed
        email: "user@example.com".into(),
        password: "password123".into(),
        age: 25,
        website: None,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn user_registration_age_below_min() {
    let user = UserRegistration {
        username: "younguser".into(),
        email: "young@example.com".into(),
        password: "password123".into(),
        age: 10,
        website: None,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn user_registration_age_above_max() {
    let user = UserRegistration {
        username: "olduser".into(),
        email: "old@example.com".into(),
        password: "password123".into(),
        age: 150,
        website: None,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn user_registration_multiple_errors() {
    let user = UserRegistration {
        username: "ab".into(),     // too short
        email: "not-email".into(), // invalid format
        password: "short".into(),  // too short
        age: 10,                   // below min
        website: None,
    };
    let errors = user.validate_fields().unwrap_err();
    assert!(errors.len() >= 4);
}

#[derive(Validator, Clone)]
struct ApiConfig {
    #[validate(url)]
    base_url: String,

    #[validate(min_length = 32, max_length = 64, regex = r"^[A-Za-z0-9]+$")]
    api_key: String,

    #[validate(min = 1, max = 300)]
    timeout_seconds: u32,

    #[validate(max = 10)]
    retry_count: u32,

    #[validate(url)]
    webhook_url: Option<String>,
}

#[test]
fn api_config_valid() {
    let config = ApiConfig {
        base_url: "https://api.example.com".into(),
        api_key: "abcdefghijklmnopqrstuvwxyz123456".into(), // 32 chars
        timeout_seconds: 30,
        retry_count: 3,
        webhook_url: Some("https://webhook.example.com/notify".into()),
    };
    assert!(config.validate_fields().is_ok());
}

#[test]
fn api_config_invalid_timeout() {
    let config = ApiConfig {
        base_url: "https://api.example.com".into(),
        api_key: "abcdefghijklmnopqrstuvwxyz123456".into(),
        timeout_seconds: 0, // below min
        retry_count: 3,
        webhook_url: None,
    };
    assert!(config.validate_fields().is_err());
}

#[test]
fn api_config_invalid_retry() {
    let config = ApiConfig {
        base_url: "https://api.example.com".into(),
        api_key: "abcdefghijklmnopqrstuvwxyz123456".into(),
        timeout_seconds: 30,
        retry_count: 15, // above max
        webhook_url: None,
    };
    assert!(config.validate_fields().is_err());
}

#[test]
fn api_config_api_key_too_short() {
    let config = ApiConfig {
        base_url: "https://api.example.com".into(),
        api_key: "short".into(), // too short
        timeout_seconds: 30,
        retry_count: 3,
        webhook_url: None,
    };
    assert!(config.validate_fields().is_err());
}

#[derive(Validator, Clone)]
struct ServerConfig {
    #[validate(hostname)]
    hostname: String,

    #[validate(ipv4)]
    ipv4_address: String,

    #[validate(ipv6)]
    ipv6_address: Option<String>,

    #[validate(min = 1, max = 65535)]
    port: u32,

    #[validate(regex = r"^(DEBUG|INFO|WARN|ERROR)$")]
    log_level: String,
}

#[test]
fn server_config_valid() {
    let config = ServerConfig {
        hostname: "api.example.com".into(),
        ipv4_address: "192.168.1.1".into(),
        ipv6_address: Some("::1".into()),
        port: 8080,
        log_level: "INFO".into(),
    };
    assert!(config.validate_fields().is_ok());
}

#[test]
fn server_config_invalid_port_zero() {
    let config = ServerConfig {
        hostname: "localhost".into(),
        ipv4_address: "127.0.0.1".into(),
        ipv6_address: None,
        port: 0, // invalid
        log_level: "DEBUG".into(),
    };
    assert!(config.validate_fields().is_err());
}

#[test]
fn server_config_invalid_port_too_high() {
    let config = ServerConfig {
        hostname: "localhost".into(),
        ipv4_address: "127.0.0.1".into(),
        ipv6_address: None,
        port: 70000, // above max
        log_level: "DEBUG".into(),
    };
    assert!(config.validate_fields().is_err());
}

#[test]
fn server_config_invalid_log_level() {
    let config = ServerConfig {
        hostname: "localhost".into(),
        ipv4_address: "127.0.0.1".into(),
        ipv6_address: None,
        port: 8080,
        log_level: "TRACE".into(), // not in allowed list
    };
    assert!(config.validate_fields().is_err());
}

// ============================================================================
// NUMERIC VALIDATORS
// ============================================================================

#[derive(Validator, Clone)]
struct IntegerBounds {
    #[validate(min = 0, max = 100)]
    percentage: i32,

    #[validate(min = 1)]
    positive_only: u32,

    #[validate(max = 1000)]
    limited: i64,
}

#[test]
fn integer_bounds_valid() {
    let i = IntegerBounds {
        percentage: 50,
        positive_only: 1,
        limited: 500,
    };
    assert!(i.validate_fields().is_ok());
}

#[test]
fn integer_bounds_at_boundaries() {
    let i = IntegerBounds {
        percentage: 0,    // exactly at min
        positive_only: 1, // exactly at min
        limited: 1000,    // exactly at max
    };
    assert!(i.validate_fields().is_ok());
}

#[test]
fn integer_bounds_percentage_negative() {
    let i = IntegerBounds {
        percentage: -1, // below 0
        positive_only: 1,
        limited: 500,
    };
    assert!(i.validate_fields().is_err());
}

#[test]
fn integer_bounds_percentage_too_high() {
    let i = IntegerBounds {
        percentage: 101, // above 100
        positive_only: 1,
        limited: 500,
    };
    assert!(i.validate_fields().is_err());
}

#[derive(Validator, Clone)]
struct FloatBounds {
    #[validate(min = 0.0, max = 1.0)]
    ratio: f64,

    #[validate(min = -273.15)]
    temperature_celsius: f64,
}

#[test]
fn float_bounds_valid() {
    let f = FloatBounds {
        ratio: 0.5,
        temperature_celsius: 20.0,
    };
    assert!(f.validate_fields().is_ok());
}

#[test]
fn float_bounds_at_boundaries() {
    let f = FloatBounds {
        ratio: 0.0,
        temperature_celsius: -273.15, // absolute zero
    };
    assert!(f.validate_fields().is_ok());
}

#[test]
fn float_bounds_ratio_negative() {
    let f = FloatBounds {
        ratio: -0.1,
        temperature_celsius: 20.0,
    };
    assert!(f.validate_fields().is_err());
}

#[test]
fn float_bounds_ratio_above_one() {
    let f = FloatBounds {
        ratio: 1.1,
        temperature_celsius: 20.0,
    };
    assert!(f.validate_fields().is_err());
}

#[test]
fn float_bounds_below_absolute_zero() {
    let f = FloatBounds {
        ratio: 0.5,
        temperature_celsius: -300.0, // below absolute zero
    };
    assert!(f.validate_fields().is_err());
}

// ============================================================================
// EDGE CASES
// ============================================================================

#[derive(Validator, Clone)]
struct EmptyStringTests {
    #[validate(email)]
    email: String,

    #[validate(url)]
    url: String,

    #[validate(hostname)]
    hostname: String,
}

#[test]
fn empty_strings_rejected() {
    let e = EmptyStringTests {
        email: "".into(),
        url: "".into(),
        hostname: "".into(),
    };
    let errors = e.validate_fields().unwrap_err();
    assert!(errors.len() >= 3);
}

#[derive(Validator, Clone)]
struct BoundaryLengths {
    #[validate(min_length = 5)]
    exactly_min: String,

    #[validate(max_length = 10)]
    exactly_max: String,

    #[validate(min_length = 3, max_length = 3)]
    exact_length: String,
}

#[test]
fn boundary_lengths_exact_valid() {
    let b = BoundaryLengths {
        exactly_min: "12345".into(),      // exactly 5
        exactly_max: "1234567890".into(), // exactly 10
        exact_length: "abc".into(),       // exactly 3
    };
    assert!(b.validate_fields().is_ok());
}

#[test]
fn boundary_lengths_one_below_min() {
    let b = BoundaryLengths {
        exactly_min: "1234".into(), // 4, below min of 5
        exactly_max: "1234567890".into(),
        exact_length: "abc".into(),
    };
    assert!(b.validate_fields().is_err());
}

#[test]
fn boundary_lengths_one_above_max() {
    let b = BoundaryLengths {
        exactly_min: "12345".into(),
        exactly_max: "12345678901".into(), // 11, above max of 10
        exact_length: "abc".into(),
    };
    assert!(b.validate_fields().is_err());
}

#[test]
fn boundary_lengths_exact_wrong() {
    let b = BoundaryLengths {
        exactly_min: "12345".into(),
        exactly_max: "1234567890".into(),
        exact_length: "ab".into(), // 2, should be exactly 3
    };
    assert!(b.validate_fields().is_err());
}

#[derive(Validator, Clone)]
struct UnicodeLengthTest {
    #[validate(min_length = 4)]
    text: String,
}

#[test]
fn unicode_length_is_byte_based() {
    // Emoji is 4 bytes in UTF-8, so exactly at min_length = 4
    let u = UnicodeLengthTest {
        text: "\u{1F980}".into(), // crab emoji
    };
    assert!(u.validate_fields().is_ok());
}

#[test]
fn unicode_multibyte_counts_bytes() {
    // Japanese character is 3 bytes each
    let u = UnicodeLengthTest {
        text: "\u{65E5}".into(), // "日" - 3 bytes, below 4
    };
    assert!(u.validate_fields().is_err());
}

// ============================================================================
// OPTIONAL FIELDS WITH MULTIPLE VALIDATORS
// ============================================================================

#[derive(Validator, Clone)]
struct RequiredOptional {
    #[validate(required, email)]
    required_email: Option<String>,
}

#[test]
fn required_optional_none_fails() {
    let r = RequiredOptional {
        required_email: None,
    };
    assert!(r.validate_fields().is_err());
}

#[test]
fn required_optional_some_valid() {
    let r = RequiredOptional {
        required_email: Some("user@example.com".into()),
    };
    assert!(r.validate_fields().is_ok());
}

#[test]
fn required_optional_some_invalid_email() {
    let r = RequiredOptional {
        required_email: Some("not-email".into()),
    };
    assert!(r.validate_fields().is_err());
}

#[derive(Validator, Clone)]
struct OptionalWithMultiple {
    #[validate(email, min_length = 10, max_length = 100)]
    contact: Option<String>,
}

#[test]
fn optional_multiple_none_passes() {
    let o = OptionalWithMultiple { contact: None };
    assert!(o.validate_fields().is_ok());
}

#[test]
fn optional_multiple_some_valid() {
    let o = OptionalWithMultiple {
        contact: Some("user@example.com".into()),
    };
    assert!(o.validate_fields().is_ok());
}

#[test]
fn optional_multiple_some_too_short() {
    let o = OptionalWithMultiple {
        contact: Some("a@b.co".into()), // valid email but too short
    };
    assert!(o.validate_fields().is_err());
}

// ============================================================================
// ERROR ACCUMULATION AND INSPECTION
// ============================================================================

#[derive(Validator, Clone)]
struct MultiErrorForm {
    #[validate(email)]
    email: String,

    #[validate(min = 18)]
    age: u32,

    #[validate(min_length = 8)]
    password: String,

    #[validate(url)]
    website: String,
}

#[test]
fn error_accumulation_all_invalid() {
    let form = MultiErrorForm {
        email: "bad".into(),
        age: 10,
        password: "short".into(),
        website: "not-url".into(),
    };
    let errors = form.validate_fields().unwrap_err();
    assert_eq!(errors.len(), 4);
}

#[test]
fn error_accumulation_partial() {
    let form = MultiErrorForm {
        email: "user@example.com".into(), // valid
        age: 10,                          // invalid
        password: "password123".into(),   // valid
        website: "not-url".into(),        // invalid
    };
    let errors = form.validate_fields().unwrap_err();
    assert_eq!(errors.len(), 2);
}

#[test]
fn errors_have_field_names() {
    let form = MultiErrorForm {
        email: "bad".into(),
        age: 25,
        password: "password123".into(),
        website: "https://example.com".into(),
    };
    let errors = form.validate_fields().unwrap_err();
    assert_eq!(errors.len(), 1);

    let error = &errors.errors()[0];
    assert_eq!(error.field.as_deref(), Some("email"));
}

// ============================================================================
// ROOT MESSAGE CUSTOMIZATION
// ============================================================================

#[derive(Validator, Clone)]
#[validator(message = "custom root error")]
struct CustomMessage {
    #[validate(email)]
    email: String,
}

#[test]
fn custom_root_message_in_validate_trait() {
    let c = CustomMessage {
        email: "invalid".into(),
    };
    let err = c.validate(&c).unwrap_err();
    assert_eq!(err.message.as_ref(), "custom root error");
}

#[derive(Validator, Clone)]
struct DefaultMessage {
    #[validate(email)]
    email: String,
}

#[test]
fn default_root_message() {
    let d = DefaultMessage {
        email: "invalid".into(),
    };
    let err = d.validate(&d).unwrap_err();
    assert_eq!(err.message.as_ref(), "validation failed");
}

// ============================================================================
// REGEX EDGE CASES
// ============================================================================

#[derive(Validator, Clone)]
struct RegexPatterns {
    #[validate(regex = r"^[A-Z][a-z]+$")]
    capitalized: String,

    #[validate(regex = r"^\d{3}-?\d{4}$")]
    phone_like: String,

    #[validate(regex = r"^(yes|no|maybe)$")]
    choice: String,
}

#[test]
fn regex_patterns_all_valid() {
    let r = RegexPatterns {
        capitalized: "Hello".into(),
        phone_like: "123-4567".into(),
        choice: "yes".into(),
    };
    assert!(r.validate_fields().is_ok());
}

#[test]
fn regex_patterns_capitalized_invalid() {
    let r = RegexPatterns {
        capitalized: "hello".into(), // should start with uppercase
        phone_like: "1234567".into(),
        choice: "no".into(),
    };
    assert!(r.validate_fields().is_err());
}

#[test]
fn regex_patterns_phone_both_formats() {
    // With dash
    let r1 = RegexPatterns {
        capitalized: "Test".into(),
        phone_like: "123-4567".into(),
        choice: "maybe".into(),
    };
    assert!(r1.validate_fields().is_ok());

    // Without dash
    let r2 = RegexPatterns {
        capitalized: "Test".into(),
        phone_like: "1234567".into(),
        choice: "maybe".into(),
    };
    assert!(r2.validate_fields().is_ok());
}

#[test]
fn regex_patterns_choice_invalid() {
    let r = RegexPatterns {
        capitalized: "Test".into(),
        phone_like: "1234567".into(),
        choice: "unknown".into(), // not in allowed values
    };
    assert!(r.validate_fields().is_err());
}
