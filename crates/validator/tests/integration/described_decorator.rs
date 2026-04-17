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
