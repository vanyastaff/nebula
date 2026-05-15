//! Integration coverage is in `tests/integration/rule_*`. Keep this
//! file for unit-level smoke tests of the new API only.

use serde_json::json;

use super::{Predicate, Rule, RuleKind, ValueRule};
use crate::foundation::FieldPath;

#[test]
fn constructors_build_value_kind() {
    assert_eq!(Rule::min_length(3).kind(), RuleKind::Value);
}

#[test]
fn constructors_build_logic_kind() {
    assert_eq!(Rule::all([Rule::min_length(3)]).kind(), RuleKind::Logic);
}

#[test]
fn described_inherits_inner_kind() {
    let r = Rule::email().with_message("bad mail");
    assert_eq!(r.kind(), RuleKind::Value);
}

#[test]
fn is_deferred_tags_custom() {
    assert!(Rule::custom("check()").is_deferred());
    assert!(!Rule::email().is_deferred());
}

#[test]
fn predicate_eq_constructor_parses_path() {
    let p = Predicate::eq("status", json!("active")).unwrap();
    assert_eq!(p.field().as_str(), "/status");
}

#[test]
fn value_rule_direct_construction_still_works() {
    let v = ValueRule::MinLength(3);
    assert!(v.validate_value(&json!("abc")).is_ok());
    assert!(v.validate_value(&json!("ab")).is_err());
    let _ = FieldPath::parse("x").unwrap(); // ensures FieldPath is still in tree
}

#[test]
fn matches_resolves_nested_pointer_paths() {
    use crate::rule::context::PredicateContext;

    // The exact case the deleted `Rule::evaluate` failed: a predicate on a
    // NESTED path. Old flat-key lookup silently returned false (fail-open).
    let rule = Rule::Predicate(Predicate::Eq(
        FieldPath::parse("/auth/mode").unwrap(),
        json!("oauth"),
    ));
    let ctx = PredicateContext::from_json(&json!({
        "auth": { "mode": "oauth" }
    }));
    assert!(
        rule.matches(&ctx),
        "nested predicate must evaluate true via PredicateContext"
    );

    let ctx_no = PredicateContext::from_json(&json!({
        "auth": { "mode": "apikey" }
    }));
    assert!(!rule.matches(&ctx_no));
}

#[test]
fn matches_value_and_deferred_are_true() {
    use crate::rule::context::PredicateContext;

    let ctx = PredicateContext::new();
    assert!(Rule::Value(ValueRule::Email).matches(&ctx));
}

#[test]
fn matches_logic_all_any_not() {
    use crate::rule::{context::PredicateContext, logic::Logic};

    let ctx = PredicateContext::from_json(&json!({"a": 1, "b": 2}));
    let a = Rule::Predicate(Predicate::Eq(FieldPath::parse("a").unwrap(), json!(1)));
    let b = Rule::Predicate(Predicate::Eq(FieldPath::parse("b").unwrap(), json!(9)));
    assert!(Rule::Logic(Box::new(Logic::Any(vec![a.clone(), b.clone()]))).matches(&ctx));
    assert!(!Rule::Logic(Box::new(Logic::All(vec![a, b.clone()]))).matches(&ctx));
    assert!(Rule::Logic(Box::new(Logic::Not(b))).matches(&ctx));
}
