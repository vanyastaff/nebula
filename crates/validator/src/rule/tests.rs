//! Integration coverage is in `tests/integration/rule_*`. Keep this
//! file for unit-level smoke tests of the new API only.

use std::assert_matches;

use serde_json::json;

use super::{
    MAX_RULE_DEPTH, Predicate, Rule, RuleBuildError, RuleKind, RuleNode, ValueRule,
    limits::RuleStats,
};
use crate::{DiagnosticDisclosure, foundation::FieldPath};

#[test]
fn constructors_build_value_kind() {
    assert_eq!(Rule::min_length(3).kind(), RuleKind::Value);
}

#[test]
fn constructors_build_logic_kind() {
    assert_eq!(
        Rule::all([Rule::min_length(3)]).unwrap().kind(),
        RuleKind::Logic
    );
}

#[test]
fn described_inherits_inner_kind() {
    let r = Rule::email().with_message("bad mail").unwrap();
    assert_eq!(r.kind(), RuleKind::Value);
}

#[test]
fn is_deferred_tags_custom() {
    assert!(Rule::custom("check()").unwrap().is_deferred());
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
    assert!(
        v.validate_value(&json!("abc"), DiagnosticDisclosure::IncludeValue)
            .is_ok()
    );
    assert!(
        v.validate_value(&json!("ab"), DiagnosticDisclosure::IncludeValue)
            .is_err()
    );
    let _ = FieldPath::parse("x").unwrap(); // ensures FieldPath is still in tree
}

#[test]
fn matches_resolves_nested_pointer_paths() {
    use crate::rule::context::PredicateContext;

    // The exact case the deleted `Rule::evaluate` failed: a predicate on a
    // NESTED path. Old flat-key lookup silently returned false (fail-open).
    let rule = Rule::predicate(Predicate::Eq(
        FieldPath::parse("/auth/mode").unwrap(),
        json!("oauth"),
    ))
    .unwrap();
    let ctx = PredicateContext::from_json(json!({
        "auth": { "mode": "oauth" }
    }));
    assert!(
        rule.matches(&ctx).unwrap(),
        "nested predicate must evaluate true via PredicateContext"
    );

    let ctx_no = PredicateContext::from_json(json!({
        "auth": { "mode": "apikey" }
    }));
    assert!(!rule.matches(&ctx_no).unwrap());
}

#[test]
fn matches_rejects_value_and_deferred_rules() {
    use crate::rule::context::PredicateContext;

    let ctx = PredicateContext::new();
    for rule in [
        Rule::value(ValueRule::Email).unwrap(),
        Rule::custom("check()").unwrap(),
    ] {
        let error = rule.matches(&ctx).unwrap_err();
        assert_eq!(error.kind(), crate::ValidationErrorKind::InvalidRule);
    }
}

#[test]
fn matches_logic_all_any_not() {
    use crate::rule::context::PredicateContext;

    let ctx = PredicateContext::from_json(json!({"a": 1, "b": 2}));
    let a = Rule::predicate(Predicate::Eq(FieldPath::parse("a").unwrap(), json!(1))).unwrap();
    let b = Rule::predicate(Predicate::Eq(FieldPath::parse("b").unwrap(), json!(9))).unwrap();
    assert!(
        Rule::any([a.clone(), b.clone()])
            .unwrap()
            .matches(&ctx)
            .unwrap()
    );
    assert!(!Rule::all([a, b.clone()]).unwrap().matches(&ctx).unwrap());
    assert!(Rule::not(b).unwrap().matches(&ctx).unwrap());
}

#[test]
fn hostile_internal_arena_is_rejected_and_drops_without_recursion() {
    const HOSTILE_NODE_COUNT: usize = 100_001;

    std::thread::Builder::new()
        .stack_size(128 * 1_024)
        .spawn(|| {
            let mut nodes = Vec::with_capacity(HOSTILE_NODE_COUNT);
            nodes.push(RuleNode::Value(ValueRule::Email));
            for child in 0..HOSTILE_NODE_COUNT - 1 {
                nodes.push(RuleNode::Not(child));
            }
            let rule = Rule {
                root: nodes.len() - 1,
                nodes,
                stats: RuleStats {
                    depth: HOSTILE_NODE_COUNT,
                    nodes: HOSTILE_NODE_COUNT,
                    operands: HOSTILE_NODE_COUNT - 1,
                    json_nodes: 0,
                    json_depth: 0,
                    text_bytes: 0,
                },
            };

            assert_matches!(
                rule.check_limits(),
                Err(RuleBuildError::DepthLimit {
                    limit: MAX_RULE_DEPTH
                })
            );
            drop(rule);
        })
        .expect("hostile-rule regression thread must start")
        .join()
        .expect("flat rule drop must not overflow the stack");
}
