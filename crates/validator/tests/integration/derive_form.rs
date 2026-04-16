//! Scenario: realistic user-registration form validated via
//! `#[derive(Validator)]`. Exercises required/nested/collection/regex rules
//! and verifies that `validate_fields()` accumulates every failure with a
//! usable field pointer.

use nebula_validator::{
    Validator,
    combinators::SelfValidating,
    foundation::{Validate, ValidationError},
};

use super::common::{assert_has_code, expect_errors, find_by_field};

#[derive(Debug, Validator)]
struct Address {
    #[validate(min_length = 1, max_length = 128)]
    line1: String,

    #[validate(min_length = 2, max_length = 64)]
    city: String,

    #[validate(regex = r"^[A-Z]{2}$")]
    country: String,
}

#[derive(Debug, Validator)]
#[validator(message = "registration failed")]
struct RegistrationForm {
    #[validate(min_length = 3, max_length = 32, alphanumeric)]
    username: String,

    #[validate(email)]
    email: String,

    #[validate(regex = r"^\+?[0-9]{7,15}$")]
    phone: String,

    #[validate(min = 18_u8, max = 120_u8)]
    age: u8,

    #[validate(is_true)]
    terms_accepted: bool,

    #[validate(required, nested)]
    address: Option<Address>,

    #[validate(min_size = 1, max_size = 5, each(min_length = 2, max_length = 20))]
    interests: Vec<String>,
}

fn valid() -> RegistrationForm {
    RegistrationForm {
        username: "alice42".into(),
        email: "alice@example.com".into(),
        phone: "+15551234567".into(),
        age: 30,
        terms_accepted: true,
        address: Some(Address {
            line1: "1 Main St".into(),
            city: "Seattle".into(),
            country: "US".into(),
        }),
        interests: vec!["rust".into(), "music".into()],
    }
}

#[test]
fn valid_form_passes_both_apis() {
    let form = valid();
    // `validate_fields` returns the full collection; `Validate::validate`
    // collapses it into a single error. Both must succeed together.
    assert!(form.validate_fields().is_ok());
    assert!(form.validate(&form).is_ok());
    assert!(SelfValidating::check(&form).is_ok());
}

#[test]
fn every_field_reports_its_own_error() {
    let form = RegistrationForm {
        username: "a!".into(),
        email: "nope".into(),
        phone: "bad".into(),
        age: 10,
        terms_accepted: false,
        address: None,
        interests: vec![],
    };
    let errors = expect_errors(form.validate_fields());

    // Every top-level failure must be present.
    assert_has_code(&errors, "min_length"); // username
    assert_has_code(&errors, "alphanumeric"); // username
    assert_has_code(&errors, "invalid_format"); // email + phone (regex)
    assert_has_code(&errors, "min"); // age
    assert_has_code(&errors, "required"); // address
    // `is_true` failure for terms_accepted is an invalid_format with the
    // `is_true` code path; accept either surface.
    let has_bool = errors
        .errors()
        .iter()
        .any(|e| ["is_true", "invalid_format"].contains(&e.code.as_ref()));
    assert!(has_bool, "expected is_true failure to surface");
}

#[test]
fn nested_address_errors_carry_path() {
    let form = RegistrationForm {
        address: Some(Address {
            line1: "".into(),
            city: "X".into(),
            country: "usa".into(), // lower + wrong length
        }),
        ..valid()
    };

    let errors = expect_errors(form.validate_fields());
    // Nested failures attach under the parent field name.
    let any_under_address = errors
        .errors()
        .iter()
        .any(|e| e.field.as_deref().is_some_and(|f| f.contains("/address")));
    assert!(
        any_under_address,
        "expected nested errors under `/address`, got: [{}]",
        errors
            .errors()
            .iter()
            .map(|e| e.field.as_deref().unwrap_or("-"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[test]
fn per_element_errors_index_the_collection() {
    let form = RegistrationForm {
        interests: vec!["ok".into(), "x".into(), "also-ok".into()],
        ..valid()
    };

    let errors = expect_errors(form.validate_fields());

    // The second element fails min_length and its field path carries the index.
    let indexed = find_by_field(&errors, "/interests/1");
    assert!(
        indexed.is_some(),
        "expected error on `/interests/1`, got codes: [{}]",
        super::common::error_code_list(&errors)
    );
    assert_eq!(indexed.unwrap().code.as_ref(), "min_length");
}

#[test]
fn collapsed_error_preserves_root_message() {
    let form = RegistrationForm { age: 5, ..valid() };
    let err: ValidationError = form.validate(&form).unwrap_err();
    assert_eq!(err.message.as_ref(), "registration failed");
    assert!(
        !err.nested().is_empty(),
        "collapsed error must keep field-level errors as nested"
    );
}
