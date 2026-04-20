//! Integration tests for the `#[derive(Validator)]` macro.
//!
//! Exercises the 3-phase pipeline (parse → emit) by deriving `Validator`
//! on real structs and verifying both happy-path and failure-path behavior.

use nebula_validator::{
    Validator,
    combinators::SelfValidating,
    foundation::{Validate, ValidationError, ValidationErrors},
};

// ============================================================================
// Helper
// ============================================================================

/// Collects error codes from a `ValidationErrors` result.
fn error_codes(result: &Result<(), ValidationErrors>) -> Vec<&str> {
    match result {
        Ok(()) => vec![],
        Err(errors) => errors.errors().iter().map(|e| e.code.as_ref()).collect(),
    }
}

/// Returns the first error message from a `ValidationErrors` result.
fn first_message(result: &Result<(), ValidationErrors>) -> &str {
    result
        .as_ref()
        .err()
        .and_then(|e| e.errors().first())
        .map_or("", |e| e.message.as_ref())
}

// ============================================================================
// 1. Basic field validation — min_length, max_length, exact_length
// ============================================================================

#[derive(Validator)]
struct BasicLengths {
    #[validate(min_length = 3)]
    name: String,
    #[validate(max_length = 10)]
    tag: String,
    #[validate(exact_length = 5)]
    code: String,
}

#[test]
fn accepts_valid_lengths() {
    let v = BasicLengths {
        name: "alice".into(),
        tag: "short".into(),
        code: "ABCDE".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_short_name() {
    let v = BasicLengths {
        name: "ab".into(),
        tag: "ok".into(),
        code: "ABCDE".into(),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"min_length"));
}

#[test]
fn rejects_long_tag() {
    let v = BasicLengths {
        name: "alice".into(),
        tag: "this-is-way-too-long".into(),
        code: "ABCDE".into(),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"max_length"));
}

#[test]
fn rejects_wrong_exact_length() {
    let v = BasicLengths {
        name: "alice".into(),
        tag: "ok".into(),
        code: "ABC".into(),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"exact_length"));
}

// ============================================================================
// 2. Required on Option
// ============================================================================

#[derive(Validator)]
struct RequiredOption {
    #[validate(required)]
    email: Option<String>,
}

#[test]
fn rejects_none_when_required() {
    let v = RequiredOption { email: None };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"required"));
}

#[test]
fn accepts_some_when_required() {
    let v = RequiredOption {
        email: Some("test@example.com".into()),
    };
    assert!(v.validate_fields().is_ok());
}

// ============================================================================
// 3. Numeric min/max
// ============================================================================

#[derive(Validator)]
struct NumericBounds {
    #[validate(min = 0)]
    score: i32,
    #[validate(max = 100)]
    percent: u32,
}

#[derive(Validator)]
struct NumericBoundsCallStyle {
    #[validate(max(100))]
    percent: u32,
}

#[test]
fn accepts_valid_numeric_bounds() {
    let v = NumericBounds {
        score: 50,
        percent: 75,
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_below_min() {
    let v = NumericBounds {
        score: -1,
        percent: 50,
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"min"));
}

#[test]
fn rejects_above_max() {
    let v = NumericBounds {
        score: 0,
        percent: 101,
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"max"));
}

#[test]
fn rejects_above_max_with_call_style() {
    let v = NumericBoundsCallStyle { percent: 101 };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"max"));
}

// ============================================================================
// 4. Boolean is_true / is_false
// ============================================================================

#[derive(Validator)]
struct BooleanChecks {
    #[validate(is_true)]
    accepted: bool,
    #[validate(is_false)]
    locked: bool,
}

#[test]
fn accepts_correct_booleans() {
    let v = BooleanChecks {
        accepted: true,
        locked: false,
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_false_when_is_true_required() {
    let v = BooleanChecks {
        accepted: false,
        locked: false,
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_true_when_is_false_required() {
    let v = BooleanChecks {
        accepted: true,
        locked: true,
    };
    assert!(v.validate_fields().is_err());
}

// ============================================================================
// 5. String format validators — email, url, not_empty
// ============================================================================

#[derive(Validator)]
struct StringFormats {
    #[validate(email)]
    email: String,
    #[validate(url)]
    website: String,
    #[validate(not_empty)]
    label: String,
}

#[test]
fn accepts_valid_string_formats() {
    let v = StringFormats {
        email: "user@example.com".into(),
        website: "https://example.com".into(),
        label: "hello".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_invalid_email() {
    let v = StringFormats {
        email: "not-an-email".into(),
        website: "https://example.com".into(),
        label: "hello".into(),
    };
    assert!(v.validate_fields().is_err());
}

#[derive(Validator)]
struct CanonicalStringFormats {
    #[validate(email())]
    email: String,
    #[validate(prefix("https://"))]
    website: String,
}

#[test]
fn canonical_string_calls_accept_valid_values() {
    let v = CanonicalStringFormats {
        email: "user@example.com".into(),
        website: "https://example.com".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn canonical_string_calls_reject_invalid_values() {
    let v = CanonicalStringFormats {
        email: "not-an-email".into(),
        website: "http://example.com".into(),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    let codes = error_codes(&result);
    assert!(
        codes.contains(&"email") || codes.contains(&"prefix") || codes.contains(&"starts_with")
    );
}

#[test]
fn rejects_invalid_url() {
    let v = StringFormats {
        email: "user@example.com".into(),
        website: "not a url".into(),
        label: "hello".into(),
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_empty_string_when_not_empty() {
    let v = StringFormats {
        email: "user@example.com".into(),
        website: "https://example.com".into(),
        label: String::new(),
    };
    assert!(v.validate_fields().is_err());
}

// ============================================================================
// 6. Regex
// ============================================================================

#[derive(Validator)]
struct RegexCheck {
    #[validate(regex = r"^[a-z]+$")]
    slug: String,
}

#[test]
fn accepts_matching_regex() {
    let v = RegexCheck {
        slug: "hello".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_non_matching_regex() {
    let v = RegexCheck {
        slug: "Hello123".into(),
    };
    assert!(v.validate_fields().is_err());
}

// ============================================================================
// 7. Length range
// ============================================================================

#[derive(Validator)]
struct LengthRangeCheck {
    #[validate(length_range(min = 3, max = 10))]
    username: String,
}

#[derive(Validator)]
struct CanonicalRangeCheck {
    #[validate(range(min = 3, max = 10))]
    username_len_like: usize,
}

#[test]
fn accepts_length_within_range() {
    let v = LengthRangeCheck {
        username: "alice".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_length_below_range() {
    let v = LengthRangeCheck {
        username: "ab".into(),
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_length_above_range() {
    let v = LengthRangeCheck {
        username: "this-is-way-too-long".into(),
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn canonical_range_rejects_out_of_bounds() {
    let v = CanonicalRangeCheck {
        username_len_like: 20,
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"max"));
}

// ============================================================================
// 8. Collection validators — min_size, max_size, exact_size, not_empty_collection, size_range
// ============================================================================

#[derive(Validator)]
struct CollectionChecks {
    #[validate(min_size = 1)]
    tags: Vec<String>,
    #[validate(max_size = 3)]
    labels: Vec<i32>,
    #[validate(exact_size = 2)]
    pair: Vec<u8>,
    #[validate(not_empty_collection)]
    items: Vec<String>,
    #[validate(size_range(min = 2, max = 5))]
    scores: Vec<f64>,
}

#[test]
fn accepts_valid_collections() {
    let v = CollectionChecks {
        tags: vec!["a".into()],
        labels: vec![1, 2],
        pair: vec![10, 20],
        items: vec!["x".into()],
        scores: vec![1.0, 2.0, 3.0],
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_too_few_tags() {
    let v = CollectionChecks {
        tags: vec![],
        labels: vec![1],
        pair: vec![10, 20],
        items: vec!["x".into()],
        scores: vec![1.0, 2.0],
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_too_many_labels() {
    let v = CollectionChecks {
        tags: vec!["a".into()],
        labels: vec![1, 2, 3, 4],
        pair: vec![10, 20],
        items: vec!["x".into()],
        scores: vec![1.0, 2.0],
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_wrong_exact_size() {
    let v = CollectionChecks {
        tags: vec!["a".into()],
        labels: vec![1],
        pair: vec![10],
        items: vec!["x".into()],
        scores: vec![1.0, 2.0],
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_empty_collection_when_not_empty() {
    let v = CollectionChecks {
        tags: vec!["a".into()],
        labels: vec![1],
        pair: vec![10, 20],
        items: vec![],
        scores: vec![1.0, 2.0],
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_size_outside_range() {
    let v = CollectionChecks {
        tags: vec!["a".into()],
        labels: vec![1],
        pair: vec![10, 20],
        items: vec!["x".into()],
        scores: vec![1.0],
    };
    assert!(v.validate_fields().is_err());
}

// ============================================================================
// 9. String factories — contains, starts_with, ends_with
// ============================================================================

#[derive(Validator)]
struct StringFactories {
    #[validate(contains = "@")]
    email_like: String,
    #[validate(starts_with = "https://")]
    secure_url: String,
    #[validate(ends_with = ".rs")]
    rust_file: String,
}

#[test]
fn accepts_valid_string_factories() {
    let v = StringFactories {
        email_like: "user@host".into(),
        secure_url: "https://example.com".into(),
        rust_file: "main.rs".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_missing_contains() {
    let v = StringFactories {
        email_like: "no-at-sign".into(),
        secure_url: "https://example.com".into(),
        rust_file: "main.rs".into(),
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_wrong_prefix() {
    let v = StringFactories {
        email_like: "user@host".into(),
        secure_url: "http://insecure.com".into(),
        rust_file: "main.rs".into(),
    };
    assert!(v.validate_fields().is_err());
}

#[test]
fn rejects_wrong_suffix() {
    let v = StringFactories {
        email_like: "user@host".into(),
        secure_url: "https://example.com".into(),
        rust_file: "main.py".into(),
    };
    assert!(v.validate_fields().is_err());
}

// ============================================================================
// 10. Nested validation
// ============================================================================

#[derive(Validator)]
struct Inner {
    #[validate(min_length = 1)]
    name: String,
}

#[derive(Validator)]
struct Outer {
    #[validate(nested)]
    inner: Inner,
}

#[derive(Validator)]
struct OuterCollection {
    #[validate(inner(nested()))]
    inner: Vec<Inner>,
}

#[test]
fn accepts_valid_nested() {
    let v = Outer {
        inner: Inner { name: "ok".into() },
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_invalid_nested() {
    let v = Outer {
        inner: Inner {
            name: String::new(),
        },
    };
    let result = v.validate_fields();
    assert!(result.is_err());
}

#[test]
fn canonical_inner_nested_rejects_invalid_nested_element() {
    let v = OuterCollection {
        inner: vec![Inner {
            name: String::new(),
        }],
    };
    let result = v.validate_fields();
    assert!(result.is_err());
}

// ============================================================================
// 11. Custom validator
// ============================================================================

fn validate_even(value: &i32) -> Result<(), ValidationError> {
    if value % 2 == 0 {
        Ok(())
    } else {
        Err(ValidationError::new("even", "must be even"))
    }
}

#[derive(Validator)]
struct CustomCheck {
    #[validate(custom = validate_even)]
    count: i32,
}

#[test]
fn accepts_custom_validator_passing() {
    let v = CustomCheck { count: 4 };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn rejects_custom_validator_failing() {
    let v = CustomCheck { count: 3 };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"even"));
}

#[derive(Validator)]
struct UsingCombinatorCheck {
    #[validate(using = ::nebula_validator::combinators::and(
        ::nebula_validator::validators::min_length(3),
        ::nebula_validator::validators::max_length(5)
    ))]
    name: String,
}

#[test]
fn using_combinator_accepts_value_inside_and_bounds() {
    let v = UsingCombinatorCheck {
        name: "alice".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn using_combinator_rejects_value_outside_and_bounds() {
    let v = UsingCombinatorCheck { name: "ab".into() };
    assert!(v.validate_fields().is_err());
}

#[derive(Validator)]
struct AllSugarCheck {
    #[validate(all(
        ::nebula_validator::validators::min_length(3),
        ::nebula_validator::validators::max_length(5)
    ))]
    name: String,
}

#[test]
fn all_sugar_accepts_when_all_pass() {
    let v = AllSugarCheck {
        name: "alice".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn all_sugar_rejects_when_one_fails() {
    let v = AllSugarCheck { name: "ab".into() };
    assert!(v.validate_fields().is_err());
}

#[derive(Validator)]
struct AnySugarCheck {
    #[validate(any(
        ::nebula_validator::validators::exact_length(3),
        ::nebula_validator::validators::exact_length(5)
    ))]
    code: String,
}

#[test]
fn any_sugar_accepts_when_any_passes() {
    let v = AnySugarCheck { code: "abc".into() };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn any_sugar_rejects_when_all_fail() {
    let v = AnySugarCheck {
        code: "abcd".into(),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"any_failed"));
}

// ============================================================================
// 12. Each() element validation
// ============================================================================

#[derive(Validator)]
struct EachCheck {
    #[validate(each(min_length = 3))]
    tags: Vec<String>,
}

#[test]
fn each_accepts_all_valid_elements() {
    let v = EachCheck {
        tags: vec!["foo".into(), "bar".into(), "baz".into()],
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn each_rejects_invalid_elements() {
    let v = EachCheck {
        tags: vec!["ok-tag".into(), "ab".into(), "x".into()],
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    // Should produce errors for elements at index 1 and 2
    let err = result.unwrap_err();
    assert_eq!(err.len(), 2);
    // Verify field paths contain indexed references
    let fields: Vec<_> = err
        .errors()
        .iter()
        .filter_map(|e| e.field.as_deref())
        .collect();
    assert!(fields.iter().any(|f| f.contains('1')));
    assert!(fields.iter().any(|f| f.contains('2')));
}

#[derive(Validator)]
struct EachOptionStringCheck {
    #[validate(each(not_empty, min_length = 2))]
    tags: Vec<Option<String>>,
}

#[test]
fn each_option_string_none_is_skipped_without_required() {
    let v = EachOptionStringCheck {
        tags: vec![None, Some("ok".into())],
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn each_option_string_some_invalid_fails() {
    let v = EachOptionStringCheck {
        tags: vec![Some(String::new())],
    };
    assert!(v.validate_fields().is_err());
}

#[derive(Validator)]
struct EachOptionBoolCheck {
    #[validate(each(required, is_true))]
    flags: Vec<Option<bool>>,
}

#[test]
fn each_option_bool_required_rejects_none() {
    let v = EachOptionBoolCheck {
        flags: vec![Some(true), None],
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"required"));
}

#[test]
fn each_option_bool_is_true_rejects_false() {
    let v = EachOptionBoolCheck {
        flags: vec![Some(false)],
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"is_true"));
}

#[derive(Validator)]
struct EachUsingCombinatorCheck {
    #[validate(each(using = ::nebula_validator::combinators::and(
        ::nebula_validator::validators::not_empty(),
        ::nebula_validator::validators::min_length(2)
    )))]
    tags: Vec<String>,
}

#[test]
fn each_using_combinator_accepts_all_elements() {
    let v = EachUsingCombinatorCheck {
        tags: vec!["ab".into(), "rust".into()],
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn each_using_combinator_rejects_invalid_element() {
    let v = EachUsingCombinatorCheck {
        tags: vec!["ab".into(), String::new()],
    };
    let result = v.validate_fields();
    assert!(result.is_err());
}

#[derive(Validator)]
struct EachAnySugarCheck {
    #[validate(each(any(
        ::nebula_validator::validators::exact_length(2),
        ::nebula_validator::validators::exact_length(4)
    )))]
    tags: Vec<String>,
}

#[test]
fn each_any_sugar_accepts_when_any_passes_per_element() {
    let v = EachAnySugarCheck {
        tags: vec!["ab".into(), "rust".into()],
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn each_any_sugar_rejects_element_when_all_fail() {
    let v = EachAnySugarCheck {
        tags: vec!["ab".into(), "abc".into()],
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"any_failed"));
}

#[derive(Validator)]
struct CanonicalLengthCheck {
    #[validate(length(6))]
    code: String,
}

#[test]
fn canonical_length_call_rejects_wrong_exact_length() {
    let v = CanonicalLengthCheck { code: "abc".into() };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"exact_length"));
}

#[derive(Validator)]
struct CanonicalInnerCheck {
    #[validate(length(min = 1), inner(length(min = 2)))]
    tags: Vec<String>,
}

#[test]
fn canonical_inner_applies_rules_to_elements() {
    let v = CanonicalInnerCheck {
        tags: vec!["ok".into(), "x".into()],
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"min_length"));
}

#[derive(Validator)]
struct CanonicalOrCheck {
    #[validate(or(length(3), length(5)))]
    code: String,
}

#[test]
fn canonical_or_accepts_when_one_branch_matches() {
    let v = CanonicalOrCheck { code: "abc".into() };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn canonical_or_rejects_when_all_branches_fail() {
    let v = CanonicalOrCheck {
        code: "abcd".into(),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"any_failed"));
}

// ============================================================================
// 13. Message override
// ============================================================================

#[derive(Validator)]
struct MessageOverride {
    #[validate(min_length = 5, message = "Name is too short, friend")]
    name: String,
}

#[test]
fn message_override_uses_custom_text() {
    let v = MessageOverride { name: "ab".into() };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert_eq!(first_message(&result), "Name is too short, friend");
}

#[test]
fn message_override_not_shown_on_success() {
    let v = MessageOverride {
        name: "alice-is-ok".into(),
    };
    assert!(v.validate_fields().is_ok());
}

// ============================================================================
// 14. Option wrapping — validator on Option<String>
// ============================================================================

#[derive(Validator)]
struct OptionalField {
    #[validate(min_length = 3)]
    nickname: Option<String>,
}

#[test]
fn option_none_passes_validation() {
    let v = OptionalField { nickname: None };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn option_some_valid_passes() {
    let v = OptionalField {
        nickname: Some("alice".into()),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn option_some_invalid_fails() {
    let v = OptionalField {
        nickname: Some("ab".into()),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    assert!(error_codes(&result).contains(&"min_length"));
}

// ============================================================================
// 15. SelfValidating trait
// ============================================================================

#[derive(Validator)]
struct SelfValidatingCheck {
    #[validate(min_length = 1)]
    value: String,
}

#[test]
fn self_validating_check_passes() {
    let v = SelfValidatingCheck { value: "ok".into() };
    assert!(SelfValidating::check(&v).is_ok());
}

#[test]
fn self_validating_check_fails() {
    let v = SelfValidatingCheck {
        value: String::new(),
    };
    assert!(SelfValidating::check(&v).is_err());
}

// ============================================================================
// 16. Validate trait — val.validate(&val)
// ============================================================================

#[derive(Validator)]
struct ValidateTraitCheck {
    #[validate(max_length = 5)]
    short: String,
}

#[test]
fn validate_trait_passes() {
    let v = ValidateTraitCheck { short: "ok".into() };
    assert!(Validate::validate(&v, &v).is_ok());
}

#[test]
fn validate_trait_fails() {
    let v = ValidateTraitCheck {
        short: "too-long-string".into(),
    };
    let result = Validate::validate(&v, &v);
    assert!(result.is_err());
}

// ============================================================================
// 17. Container message
// ============================================================================

#[derive(Validator)]
#[validator(message = "user profile is invalid")]
struct ContainerMessage {
    #[validate(min_length = 3)]
    name: String,
}

#[test]
fn container_message_appears_in_single_error() {
    let v = ContainerMessage { name: "ab".into() };
    // SelfValidating::check / Validate::validate produce a single error
    // whose message is the container message.
    let result = SelfValidating::check(&v);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.message.as_ref(), "user profile is invalid");
}

// ============================================================================
// Combined: multiple rules on one field
// ============================================================================

#[derive(Validator)]
struct MultipleRules {
    #[validate(min_length = 2, max_length = 10, not_empty)]
    username: String,
}

#[test]
fn multiple_rules_all_pass() {
    let v = MultipleRules {
        username: "alice".into(),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn multiple_rules_collects_all_errors() {
    // Empty string violates both min_length and not_empty
    let v = MultipleRules {
        username: String::new(),
    };
    let result = v.validate_fields();
    assert!(result.is_err());
    let codes = error_codes(&result);
    assert!(codes.contains(&"min_length"));
}

// ============================================================================
// Combined: nested with Option
// ============================================================================

#[derive(Validator)]
struct InnerForNested {
    #[validate(not_empty)]
    label: String,
}

#[derive(Validator)]
struct OuterOptionalNested {
    #[validate(nested)]
    child: Option<InnerForNested>,
}

#[test]
fn optional_nested_none_passes() {
    let v = OuterOptionalNested { child: None };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn optional_nested_some_valid_passes() {
    let v = OuterOptionalNested {
        child: Some(InnerForNested { label: "ok".into() }),
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn optional_nested_some_invalid_fails() {
    let v = OuterOptionalNested {
        child: Some(InnerForNested {
            label: String::new(),
        }),
    };
    assert!(v.validate_fields().is_err());
}

// ---------------------------------------------------------------------------
// Exclusive numeric bounds: greater_than / less_than
// ---------------------------------------------------------------------------

#[derive(Validator)]
struct ExclusiveBounds {
    #[validate(greater_than = 0_i32)]
    positive_count: i32,

    #[validate(less_than = 100_u32)]
    under_hundred: u32,

    #[validate(greater_than = 0.0_f64, less_than = 1.0_f64)]
    probability: f64,
}

#[test]
fn greater_than_derive_passes_above_bound() {
    let v = ExclusiveBounds {
        positive_count: 1,
        under_hundred: 0,
        probability: 0.5,
    };
    assert!(v.validate_fields().is_ok());
}

#[test]
fn greater_than_derive_rejects_bound_value() {
    let v = ExclusiveBounds {
        positive_count: 0, // fails: bound is exclusive
        under_hundred: 99,
        probability: 0.5,
    };
    let errs = v.validate_fields().unwrap_err();
    assert!(
        errs.errors()
            .iter()
            .any(|e| e.code.as_ref() == "greater_than")
    );
}

#[test]
fn less_than_derive_rejects_bound_value() {
    let v = ExclusiveBounds {
        positive_count: 5,
        under_hundred: 100, // fails: bound is exclusive
        probability: 0.5,
    };
    let errs = v.validate_fields().unwrap_err();
    assert!(errs.errors().iter().any(|e| e.code.as_ref() == "less_than"));
}

#[test]
fn exclusive_bounds_float_boundary() {
    // Both 0.0 and 1.0 must fail because bounds are exclusive.
    let v = ExclusiveBounds {
        positive_count: 1,
        under_hundred: 0,
        probability: 0.0,
    };
    assert!(v.validate_fields().is_err());

    let v = ExclusiveBounds {
        positive_count: 1,
        under_hundred: 0,
        probability: 1.0,
    };
    assert!(v.validate_fields().is_err());
}

#[derive(Validator)]
struct ExclusiveBoundsEach {
    #[validate(each(greater_than = 0_i32, less_than = 10_i32))]
    values: Vec<i32>,
}

#[test]
fn exclusive_bounds_on_each_element() {
    let valid = ExclusiveBoundsEach {
        values: vec![1, 5, 9],
    };
    assert!(valid.validate_fields().is_ok());

    let with_zero = ExclusiveBoundsEach { values: vec![0, 5] };
    assert!(with_zero.validate_fields().is_err());

    let with_ten = ExclusiveBoundsEach {
        values: vec![5, 10],
    };
    assert!(with_ten.validate_fields().is_err());
}
