//! Integration tests for the static validation engine.
//!
//! Covers: ValidationProfile (Strict / Warn / Permissive),
//! conditional-required, declarative rules, nested object validation,
//! and the ValidationReport.

use nebula_parameter::{
    Condition, Field, FieldMetadata, OptionSource, Rule, Schema, SelectOption, ValidationProfile,
};
use serde_json::json;

fn make_values(pairs: &[(&str, serde_json::Value)]) -> nebula_parameter::ParameterValues {
    let mut v = nebula_parameter::ParameterValues::new();
    for (k, val) in pairs {
        v.set(*k, val.clone());
    }
    v
}

// ── ValidationProfile::Strict ────────────────────────────────────────────────

#[test]
fn strict_unknown_field_is_error() {
    let schema = Schema::new().field(Field::text("name").with_label("Name"));
    let values = make_values(&[("name", json!("Alice")), ("extra", json!(true))]);

    let report = schema.validate_with_profile(&values, ValidationProfile::Strict);
    assert!(report.has_errors());
    assert!(report.errors.iter().any(|e| matches!(
        e,
        nebula_parameter::ParameterError::UnknownField { key } if key == "extra"
    )));
}

#[test]
fn strict_valid_input_passes() {
    let schema = Schema::new()
        .field(Field::text("name").with_label("Name").required())
        .field(Field::integer("age").with_label("Age"));
    let values = make_values(&[("name", json!("Alice")), ("age", json!(30))]);

    assert!(schema.validate(&values).is_ok());
}

// ── ValidationProfile::Warn ──────────────────────────────────────────────────

#[test]
fn warn_profile_unknown_field_is_warning_not_error() {
    let schema = Schema::new().field(Field::text("name").with_label("Name"));
    let values = make_values(&[("name", json!("Bob")), ("ghost", json!("boo"))]);

    let report = schema.validate_with_profile(&values, ValidationProfile::Warn);
    assert!(!report.has_errors(), "should have no errors");
    assert!(report.has_warnings(), "should have a warning");
    assert!(report.warnings.iter().any(|w| matches!(
        w,
        nebula_parameter::ParameterError::UnknownField { key } if key == "ghost"
    )));
}

// ── ValidationProfile::Permissive ────────────────────────────────────────────

#[test]
fn permissive_profile_unknown_field_is_silent() {
    let schema = Schema::new().field(Field::text("name").with_label("Name"));
    let values = make_values(&[
        ("name", json!("Carol")),
        ("mystery_key", json!(42)),
        ("another_mystery", json!(null)),
    ]);

    let report = schema.validate_with_profile(&values, ValidationProfile::Permissive);
    assert!(!report.has_errors());
    assert!(!report.has_warnings());
}

// ── Required fields ──────────────────────────────────────────────────────────

#[test]
fn missing_required_field_is_error() {
    let schema = Schema::new().field(Field::text("api_key").with_label("API Key").required());
    let values = make_values(&[]);

    let errors = schema.validate(&values).unwrap_err();
    assert!(errors.iter().any(|e| matches!(
        e,
        nebula_parameter::ParameterError::MissingValue { key } if key == "api_key"
    )));
}

#[test]
fn required_when_condition_true_and_value_absent_is_error() {
    let schema = Schema::new()
        .field(Field::text("auth_mode").with_label("Auth Mode"))
        .field(
            Field::text("token")
                .with_label("Token")
                .required_when(Condition::Eq {
                    field: "auth_mode".to_owned(),
                    value: json!("bearer"),
                }),
        );

    // auth_mode = "bearer" but token is absent
    let values = make_values(&[("auth_mode", json!("bearer"))]);
    assert!(schema.validate(&values).is_err());
}

#[test]
fn required_when_condition_false_and_value_absent_passes() {
    let schema = Schema::new()
        .field(Field::text("auth_mode").with_label("Auth Mode"))
        .field(
            Field::text("token")
                .with_label("Token")
                .required_when(Condition::Eq {
                    field: "auth_mode".to_owned(),
                    value: json!("bearer"),
                }),
        );

    // auth_mode = "none" — condition is false, so token is not required
    let values = make_values(&[("auth_mode", json!("none"))]);
    assert!(schema.validate(&values).is_ok());
}

// ── Declarative rules ────────────────────────────────────────────────────────

#[test]
fn min_length_rule_rejects_short_value() {
    let schema = Schema::new().field(Field::text("username").with_label("Username").with_rule(
        Rule::MinLength {
            min: 5,
            message: None,
        },
    ));
    let values = make_values(&[("username", json!("ab"))]);

    assert!(schema.validate(&values).is_err());
}

#[test]
fn min_length_rule_accepts_long_enough_value() {
    let schema = Schema::new().field(Field::text("username").with_label("Username").with_rule(
        Rule::MinLength {
            min: 3,
            message: None,
        },
    ));
    let values = make_values(&[("username", json!("alice"))]);

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn max_length_rule_rejects_too_long_value() {
    let schema = Schema::new().field(Field::text("code").with_label("Code").with_rule(
        Rule::MaxLength {
            max: 4,
            message: None,
        },
    ));
    let values = make_values(&[("code", json!("TOOLONG"))]);

    assert!(schema.validate(&values).is_err());
}

#[test]
fn pattern_rule_rejects_non_matching_value() {
    let schema = Schema::new().field(Field::text("slug").with_label("Slug").with_rule(
        Rule::Pattern {
            pattern: r"^[a-z0-9\-]+$".to_owned(),
            message: Some("only lowercase letters, digits and hyphens".to_owned()),
        },
    ));
    let values = make_values(&[("slug", json!("Hello World!"))]);

    assert!(schema.validate(&values).is_err());
}

#[test]
fn pattern_rule_accepts_matching_value() {
    let schema = Schema::new().field(Field::text("slug").with_label("Slug").with_rule(
        Rule::Pattern {
            pattern: r"^[a-z0-9\-]+$".to_owned(),
            message: None,
        },
    ));
    let values = make_values(&[("slug", json!("my-cool-slug"))]);

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn one_of_rule_rejects_unlisted_value() {
    let schema = Schema::new().field(Field::text("env").with_label("Environment").with_rule(
        Rule::OneOf {
            values: vec![json!("dev"), json!("staging"), json!("prod")],
            message: None,
        },
    ));
    let values = make_values(&[("env", json!("local"))]);

    assert!(schema.validate(&values).is_err());
}

#[test]
fn one_of_rule_accepts_listed_value() {
    let schema = Schema::new().field(Field::text("env").with_label("Environment").with_rule(
        Rule::OneOf {
            values: vec![json!("dev"), json!("staging"), json!("prod")],
            message: None,
        },
    ));
    let values = make_values(&[("env", json!("prod"))]);

    assert!(schema.validate(&values).is_ok());
}

// ── Static select membership ─────────────────────────────────────────────────

#[test]
fn static_select_rejects_value_not_in_options() {
    let schema = Schema::new().field(Field::Select {
        meta: {
            let mut m = FieldMetadata::new("method");
            m.set_label("HTTP Method");
            m
        },
        source: OptionSource::Static {
            options: vec![
                SelectOption::new(json!("GET"), "GET"),
                SelectOption::new(json!("POST"), "POST"),
            ],
        },
        multiple: false,
        allow_custom: false,
        searchable: false,
        loader: None,
    });
    let values = make_values(&[("method", json!("PATCH"))]);

    assert!(schema.validate(&values).is_err());
}

#[test]
fn static_select_allow_custom_accepts_any_value() {
    let schema = Schema::new().field(Field::Select {
        meta: {
            let mut m = FieldMetadata::new("tag");
            m.set_label("Tag");
            m
        },
        source: OptionSource::Static {
            options: vec![SelectOption::new(json!("featured"), "Featured")],
        },
        multiple: false,
        allow_custom: true,
        searchable: false,
        loader: None,
    });
    let values = make_values(&[("tag", json!("custom-tag"))]);

    assert!(schema.validate(&values).is_ok());
}

// ── Nested object validation ─────────────────────────────────────────────────

#[test]
fn nested_required_field_missing_reports_dotted_path() {
    let schema = Schema::new().field(Field::Object {
        meta: FieldMetadata::new("auth"),
        fields: vec![Field::text("token").with_label("Token").required()],
    });
    let values = make_values(&[("auth", json!({}))]);

    let errors = schema.validate(&values).unwrap_err();
    assert!(errors.iter().any(|e| matches!(
        e,
        nebula_parameter::ParameterError::MissingValue { key } if key == "auth.token"
    )));
}

#[test]
fn nested_unknown_field_reports_dotted_path() {
    let schema = Schema::new().field(Field::Object {
        meta: FieldMetadata::new("config"),
        fields: vec![Field::text("host").with_label("Host")],
    });
    let values = make_values(&[("config", json!({ "host": "localhost", "port": 5432 }))]);

    let errors = schema.validate(&values).unwrap_err();
    assert!(errors.iter().any(|e| matches!(
        e,
        nebula_parameter::ParameterError::UnknownField { key } if key == "config.port"
    )));
}

// ── ValidationReport helpers ─────────────────────────────────────────────────

#[test]
fn report_is_ok_with_no_issues() {
    let schema = Schema::new().field(Field::text("x").with_label("X"));
    let values = make_values(&[("x", json!("hello"))]);
    let report = schema.validate_with_profile(&values, ValidationProfile::Strict);

    assert!(report.is_ok());
    assert!(!report.has_errors());
    assert!(!report.has_warnings());
}

#[test]
fn report_into_result_returns_err_when_errors_exist() {
    let schema = Schema::new().field(Field::text("x").with_label("X").required());
    let values = make_values(&[]);
    let report = schema.validate_with_profile(&values, ValidationProfile::Strict);

    assert!(report.into_result().is_err());
}
