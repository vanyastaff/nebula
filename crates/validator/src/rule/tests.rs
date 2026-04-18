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
