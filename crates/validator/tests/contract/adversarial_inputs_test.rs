use nebula_validator::combinators::each;
use nebula_validator::foundation::Validate;
use nebula_validator::validators::{matches_regex, min};

#[test]
fn regex_heavy_payloads_remain_deterministic() {
    let validator = matches_regex(r"^[a-z0-9_-]{3,64}$").expect("regex must compile");

    let long_valid = "a".repeat(64);
    let long_invalid = format!("{}!", "a".repeat(63));

    assert!(validator.validate(&long_valid).is_ok());
    let err = validator
        .validate(&long_invalid)
        .expect_err("invalid char must fail deterministically");
    assert_eq!(err.code.as_ref(), "invalid_format");
}

#[test]
fn nested_collection_failures_are_bounded_and_parseable() {
    let validator = each(min(3i64));
    let payload = vec![10, 1, 2, 9, 0];

    let err = validator
        .validate(payload.as_slice())
        .expect_err("several entries are too short");
    assert!(err.total_error_count() <= payload.len() + 1);
    assert!(err.has_nested());
}
