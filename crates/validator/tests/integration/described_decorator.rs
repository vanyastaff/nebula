//! Scenario: `Described(Box<Rule>, String)` wraps any Rule (including
//! nested Described and Logic), overrides the resulting message, and
//! preserves the error's code and field context.

use nebula_validator::{DiagnosticDisclosure, ExecutionMode, Rule};
use serde_json::{Value, json};

fn validate_disclosed(
    rule: &Rule,
    value: &Value,
) -> Result<nebula_validator::EvaluationOutcome, nebula_validator::foundation::ValidationError> {
    rule.validate(
        value,
        None,
        ExecutionMode::Full,
        DiagnosticDisclosure::IncludeValue,
    )
}

#[test]
fn described_overrides_leaf_message() {
    let rule = Rule::min_length(3).with_message("too short").unwrap();
    let err = validate_disclosed(&rule, &json!("ab")).unwrap_err();
    assert_eq!(err.message.as_ref(), "too short");
    assert_eq!(err.code.as_ref(), "min_length");
}

#[test]
fn described_wraps_combinator() {
    let rule = Rule::all([Rule::min_length(3), Rule::pattern("^[a-z]+$").unwrap()])
        .unwrap()
        .with_message("combined fail")
        .unwrap();
    let err = validate_disclosed(&rule, &json!("A")).unwrap_err();
    assert_eq!(err.message.as_ref(), "combined fail");
    // Both inner rules fail, so `Logic::All` aggregates them into the
    // `all_failed` parent code — Described overrides only message, not code.
    assert_eq!(err.code.as_ref(), "all_failed");
}

#[test]
fn described_preserves_leaf_code_when_only_one_inner_fails() {
    // Single failing inner rule bypasses the `all_failed` wrapper in
    // Logic::All (see logic.rs: if errs.len() == 1 the sole error is
    // returned directly). Described then overlays the message without
    // touching that preserved code.
    let rule = Rule::all([Rule::min_length(3), Rule::pattern("^[a-z]+$").unwrap()])
        .unwrap()
        .with_message("combined fail")
        .unwrap();
    // "ab" passes pattern but fails min_length → single inner error.
    let err = validate_disclosed(&rule, &json!("ab")).unwrap_err();
    assert_eq!(err.message.as_ref(), "combined fail");
    assert_eq!(err.code.as_ref(), "min_length");
}

#[test]
fn described_template_renders_eagerly_in_message() {
    // PR contract: err.message contains the rendered string, not the raw
    // template. Consumers reading err.message directly (e.g. JSON output)
    // should see substituted placeholders.
    let rule = Rule::min_length(3)
        .with_message("got {value}, need {min}")
        .unwrap();
    let err = validate_disclosed(&rule, &json!("x")).unwrap_err();
    assert!(
        err.message.contains("got \"x\""),
        "expected rendered {{value}} in err.message, got: {}",
        err.message
    );
    assert!(
        err.message.contains("need 3"),
        "expected rendered {{min}} in err.message, got: {}",
        err.message
    );
}

#[test]
fn outer_described_wins_over_inner() {
    let inner = Rule::min_length(3).with_message("inner").unwrap();
    let outer = inner.with_message("outer").unwrap();
    let err = validate_disclosed(&outer, &json!("a")).unwrap_err();
    assert_eq!(err.message.as_ref(), "outer");
}

#[test]
fn described_does_not_change_passing_rule() {
    let rule = Rule::min_length(3).with_message("err text").unwrap();
    assert!(validate_disclosed(&rule, &json!("hello")).is_ok());
}

#[test]
fn described_kind_follows_inner() {
    use nebula_validator::RuleKind;
    let r = Rule::email().with_message("x").unwrap();
    assert_eq!(r.kind(), RuleKind::Value);
}
