//! Scenario: `{name}` placeholders in `Described` messages render from
//! ValidationError params at Display time.

use nebula_validator::{DiagnosticDisclosure, ExecutionMode, Rule};
use serde_json::{Value, json};

fn validation_error(rule: &Rule, value: &Value) -> nebula_validator::foundation::ValidationError {
    rule.validate(
        value,
        None,
        ExecutionMode::Full,
        DiagnosticDisclosure::IncludeValue,
    )
    .unwrap_err()
}

#[test]
fn min_placeholder_renders() {
    let rule = Rule::min_length(3)
        .with_message("need at least {min} chars")
        .unwrap();
    let err = validation_error(&rule, &json!("a"));
    let rendered = format!("{err}");
    assert!(
        rendered.contains("need at least 3 chars"),
        "got: {rendered}"
    );
}

#[test]
fn multiple_placeholders() {
    let rule = Rule::min_length(3)
        .with_message("got {value}, need {min}")
        .unwrap();
    let err = validation_error(&rule, &json!("hi"));
    let rendered = format!("{err}");
    assert!(rendered.contains("got \"hi\""), "got: {rendered}");
    assert!(rendered.contains("need 3"), "got: {rendered}");
}

#[test]
fn pattern_placeholder() {
    let rule = Rule::pattern("^[0-9]+$")
        .unwrap()
        .with_message("does not match {pattern}")
        .unwrap();
    let err = validation_error(&rule, &json!("abc"));
    let rendered = format!("{err}");
    assert!(rendered.contains("^[0-9]+$"), "got: {rendered}");
}

#[test]
fn unknown_placeholder_left_literal() {
    let rule = Rule::min_length(3)
        .with_message("value is {mystery_field}")
        .unwrap();
    let err = validation_error(&rule, &json!("a"));
    let rendered = format!("{err}");
    assert!(rendered.contains("{mystery_field}"), "got: {rendered}");
}

#[test]
fn escape_double_brace() {
    let rule = Rule::min_length(3)
        .with_message("needs {{}} brackets")
        .unwrap();
    let err = validation_error(&rule, &json!("a"));
    let rendered = format!("{err}");
    assert!(rendered.contains("needs {} brackets"), "got: {rendered}");
}
