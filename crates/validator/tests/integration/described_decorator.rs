//! Scenario: `Described(Box<Rule>, String)` wraps any Rule (including
//! nested Described and Logic), overrides the resulting message, and
//! preserves the error's code and field context.

use nebula_validator::{Rule, foundation::Validate};
use serde_json::json;

#[test]
fn described_overrides_leaf_message() {
    let rule = Rule::min_length(3).with_message("too short");
    let err = Validate::validate(&rule, &json!("ab")).unwrap_err();
    assert_eq!(err.message.as_ref(), "too short");
    assert_eq!(err.code.as_ref(), "min_length");
}

#[test]
fn described_wraps_combinator() {
    let rule =
        Rule::all([Rule::min_length(3), Rule::pattern("^[a-z]+$")]).with_message("combined fail");
    let err = Validate::validate(&rule, &json!("A")).unwrap_err();
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
    let rule =
        Rule::all([Rule::min_length(3), Rule::pattern("^[a-z]+$")]).with_message("combined fail");
    // "ab" passes pattern but fails min_length → single inner error.
    let err = Validate::validate(&rule, &json!("ab")).unwrap_err();
    assert_eq!(err.message.as_ref(), "combined fail");
    assert_eq!(err.code.as_ref(), "min_length");
}

#[test]
fn described_template_renders_eagerly_in_message() {
    // PR contract: err.message contains the rendered string, not the raw
    // template. Consumers reading err.message directly (e.g. JSON output)
    // should see substituted placeholders.
    let rule = Rule::min_length(3).with_message("got {value}, need {min}");
    let err = Validate::validate(&rule, &json!("x")).unwrap_err();
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
    let inner = Rule::min_length(3).with_message("inner");
    let outer = inner.with_message("outer");
    let err = Validate::validate(&outer, &json!("a")).unwrap_err();
    assert_eq!(err.message.as_ref(), "outer");
}

#[test]
fn described_does_not_change_passing_rule() {
    let rule = Rule::min_length(3).with_message("err text");
    assert!(Validate::validate(&rule, &json!("hello")).is_ok());
}

#[test]
fn described_kind_follows_inner() {
    use nebula_validator::RuleKind;
    let r = Rule::email().with_message("x");
    assert_eq!(r.kind(), RuleKind::Value);
}
