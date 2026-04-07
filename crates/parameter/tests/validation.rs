//! Integration tests for the static validation engine.
//!
//! Covers: ValidationProfile (Strict / Warn / Permissive),
//! conditional-required, declarative rules, nested object validation,
//! and the ValidationReport.

use nebula_parameter::{
    Condition, Parameter, ParameterCollection, ParameterValues, Rule, ValidationProfile,
};
use serde_json::json;

fn make_values(pairs: &[(&str, serde_json::Value)]) -> ParameterValues {
    let mut v = ParameterValues::new();
    for (k, val) in pairs {
        v.set(*k, val.clone());
    }
    v
}

// ── ValidationProfile::Strict ────────────────────────────────────────────────

#[test]
fn strict_unknown_field_is_error() {
    let collection = ParameterCollection::new().add(Parameter::string("name").label("Name"));
    let values = make_values(&[("name", json!("Alice")), ("extra", json!(true))]);

    let report = collection.validate_with_profile(&values, ValidationProfile::Strict);
    assert!(report.has_errors());
    assert!(report.errors().iter().any(|e| matches!(
        e,
        nebula_parameter::ParameterError::UnknownField { key } if key == "extra"
    )));
}

#[test]
fn strict_valid_input_passes() {
    let collection = ParameterCollection::new()
        .add(Parameter::string("name").label("Name").required())
        .add(Parameter::integer("age").label("Age"));
    let values = make_values(&[("name", json!("Alice")), ("age", json!(30))]);

    assert!(collection.validate(&values).is_ok());
}

// ── ValidationProfile::Warn ──────────────────────────────────────────────────

#[test]
fn warn_profile_unknown_field_is_warning_not_error() {
    let collection = ParameterCollection::new().add(Parameter::string("name").label("Name"));
    let values = make_values(&[("name", json!("Bob")), ("ghost", json!("boo"))]);

    let report = collection.validate_with_profile(&values, ValidationProfile::Warn);
    assert!(!report.has_errors(), "should have no errors");
    assert!(report.has_warnings(), "should have a warning");
    assert!(report.warnings().iter().any(|w| matches!(
        w,
        nebula_parameter::ParameterError::UnknownField { key } if key == "ghost"
    )));
}

// ── ValidationProfile::Permissive ────────────────────────────────────────────

#[test]
fn permissive_profile_unknown_field_is_silent() {
    let collection = ParameterCollection::new().add(Parameter::string("name").label("Name"));
    let values = make_values(&[
        ("name", json!("Carol")),
        ("mystery_key", json!(42)),
        ("another_mystery", json!(null)),
    ]);

    let report = collection.validate_with_profile(&values, ValidationProfile::Permissive);
    assert!(!report.has_errors());
    assert!(!report.has_warnings());
}

// ── Required fields ──────────────────────────────────────────────────────────

#[test]
fn missing_required_field_is_error() {
    let collection =
        ParameterCollection::new().add(Parameter::string("api_key").label("API Key").required());
    let values = make_values(&[]);

    let errors = collection.validate(&values).unwrap_err();
    assert!(errors.iter().any(|e| matches!(
        e,
        nebula_parameter::ParameterError::MissingValue { key } if key == "api_key"
    )));
}

#[test]
fn required_when_condition_true_and_value_absent_is_error() {
    let collection = ParameterCollection::new()
        .add(Parameter::string("auth_mode").label("Auth Mode"))
        .add(
            Parameter::string("token")
                .label("Token")
                .required_when(Condition::eq("auth_mode", "bearer")),
        );

    // auth_mode = "bearer" but token is absent
    let values = make_values(&[("auth_mode", json!("bearer"))]);
    assert!(collection.validate(&values).is_err());
}

#[test]
fn required_when_condition_false_and_value_absent_passes() {
    let collection = ParameterCollection::new()
        .add(Parameter::string("auth_mode").label("Auth Mode"))
        .add(
            Parameter::string("token")
                .label("Token")
                .required_when(Condition::eq("auth_mode", "bearer")),
        );

    // auth_mode = "none" — condition is false, so token is not required
    let values = make_values(&[("auth_mode", json!("none"))]);
    assert!(collection.validate(&values).is_ok());
}

// ── Declarative rules ────────────────────────────────────────────────────────

#[test]
fn min_length_rule_rejects_short_value() {
    let collection =
        ParameterCollection::new().add(Parameter::string("username").label("Username").with_rule(
            Rule::MinLength {
                min: 5,
                message: None,
            },
        ));
    let values = make_values(&[("username", json!("ab"))]);

    assert!(collection.validate(&values).is_err());
}

#[test]
fn min_length_rule_accepts_long_enough_value() {
    let collection =
        ParameterCollection::new().add(Parameter::string("username").label("Username").with_rule(
            Rule::MinLength {
                min: 3,
                message: None,
            },
        ));
    let values = make_values(&[("username", json!("alice"))]);

    assert!(collection.validate(&values).is_ok());
}

#[test]
fn max_length_rule_rejects_too_long_value() {
    let collection =
        ParameterCollection::new().add(Parameter::string("code").label("Code").with_rule(
            Rule::MaxLength {
                max: 4,
                message: None,
            },
        ));
    let values = make_values(&[("code", json!("TOOLONG"))]);

    assert!(collection.validate(&values).is_err());
}

#[test]
fn pattern_rule_rejects_non_matching_value() {
    let collection =
        ParameterCollection::new().add(Parameter::string("slug").label("Slug").with_rule(
            Rule::Pattern {
                pattern: r"^[a-z0-9\-]+$".to_owned(),
                message: Some("only lowercase letters, digits and hyphens".to_owned()),
            },
        ));
    let values = make_values(&[("slug", json!("Hello World!"))]);

    assert!(collection.validate(&values).is_err());
}

#[test]
fn pattern_rule_accepts_matching_value() {
    let collection =
        ParameterCollection::new().add(Parameter::string("slug").label("Slug").with_rule(
            Rule::Pattern {
                pattern: r"^[a-z0-9\-]+$".to_owned(),
                message: None,
            },
        ));
    let values = make_values(&[("slug", json!("my-cool-slug"))]);

    assert!(collection.validate(&values).is_ok());
}

#[test]
fn one_of_rule_rejects_unlisted_value() {
    let collection =
        ParameterCollection::new().add(Parameter::string("env").label("Environment").with_rule(
            Rule::OneOf {
                values: vec![json!("dev"), json!("staging"), json!("prod")],
                message: None,
            },
        ));
    let values = make_values(&[("env", json!("local"))]);

    assert!(collection.validate(&values).is_err());
}

#[test]
fn one_of_rule_accepts_listed_value() {
    let collection =
        ParameterCollection::new().add(Parameter::string("env").label("Environment").with_rule(
            Rule::OneOf {
                values: vec![json!("dev"), json!("staging"), json!("prod")],
                message: None,
            },
        ));
    let values = make_values(&[("env", json!("prod"))]);

    assert!(collection.validate(&values).is_ok());
}

// ── Static select membership ─────────────────────────────────────────────────

#[test]
fn static_select_rejects_value_not_in_options() {
    let collection = ParameterCollection::new().add(
        Parameter::select("method")
            .label("HTTP Method")
            .option(json!("GET"), "GET")
            .option(json!("POST"), "POST"),
    );
    let values = make_values(&[("method", json!("PATCH"))]);

    assert!(collection.validate(&values).is_err());
}

#[test]
fn static_select_allow_custom_accepts_any_value() {
    let collection = ParameterCollection::new().add(
        Parameter::select("tag")
            .label("Tag")
            .option(json!("featured"), "Featured")
            .allow_custom(),
    );
    let values = make_values(&[("tag", json!("custom-tag"))]);

    assert!(collection.validate(&values).is_ok());
}

// ── Nested object validation ─────────────────────────────────────────────────

#[test]
fn nested_required_field_missing_reports_dotted_path() {
    let collection = ParameterCollection::new()
        .add(Parameter::object("auth").add(Parameter::string("token").label("Token").required()));
    let values = make_values(&[("auth", json!({}))]);

    let errors = collection.validate(&values).unwrap_err();
    assert!(errors.iter().any(|e| matches!(
        e,
        nebula_parameter::ParameterError::MissingValue { key } if key == "auth.token"
    )));
}

#[test]
fn nested_unknown_field_reports_dotted_path() {
    let collection = ParameterCollection::new()
        .add(Parameter::object("config").add(Parameter::string("host").label("Host")));
    let values = make_values(&[("config", json!({ "host": "localhost", "port": 5432 }))]);

    // Unknown fields inside nested objects are reported as warnings.
    let report = collection.validate_with_profile(&values, ValidationProfile::Strict);
    assert!(report.warnings().iter().any(|w| matches!(
        w,
        nebula_parameter::ParameterError::UnknownField { key } if key == "config.port"
    )));
}

// ── ValidationReport helpers ─────────────────────────────────────────────────

#[test]
fn report_is_ok_with_no_issues() {
    let collection = ParameterCollection::new().add(Parameter::string("x").label("X"));
    let values = make_values(&[("x", json!("hello"))]);
    let report = collection.validate_with_profile(&values, ValidationProfile::Strict);

    assert!(report.is_ok());
    assert!(!report.has_errors());
    assert!(!report.has_warnings());
}

#[test]
fn report_into_result_returns_err_when_errors_exist() {
    let collection = ParameterCollection::new().add(Parameter::string("x").label("X").required());
    let values = make_values(&[]);
    let report = collection.validate_with_profile(&values, ValidationProfile::Strict);

    assert!(report.into_result().is_err());
}
