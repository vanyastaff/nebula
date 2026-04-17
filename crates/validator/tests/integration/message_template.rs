//! Scenario: `{name}` placeholders in `Described` messages render from
//! ValidationError params at Display time.

use nebula_validator::{Rule, foundation::Validate};
use serde_json::json;

#[test]
fn min_placeholder_renders() {
    let rule = Rule::min_length(3).with_message("need at least {min} chars");
    let err = Validate::validate(&rule, &json!("a")).unwrap_err();
    let rendered = format!("{err}");
    assert!(
        rendered.contains("need at least 3 chars"),
        "got: {rendered}"
    );
}

#[test]
fn multiple_placeholders() {
    let rule = Rule::min_length(3).with_message("got {value}, need {min}");
    let err = Validate::validate(&rule, &json!("hi")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("got \"hi\""), "got: {rendered}");
    assert!(rendered.contains("need 3"), "got: {rendered}");
}

#[test]
fn pattern_placeholder() {
    let rule = Rule::pattern("^[0-9]+$").with_message("does not match {pattern}");
    let err = Validate::validate(&rule, &json!("abc")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("^[0-9]+$"), "got: {rendered}");
}

#[test]
fn unknown_placeholder_left_literal() {
    let rule = Rule::min_length(3).with_message("value is {mystery_field}");
    let err = Validate::validate(&rule, &json!("a")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("{mystery_field}"), "got: {rendered}");
}

#[test]
fn escape_double_brace() {
    let rule = Rule::min_length(3).with_message("needs {{}} brackets");
    let err = Validate::validate(&rule, &json!("a")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("needs {} brackets"), "got: {rendered}");
}
