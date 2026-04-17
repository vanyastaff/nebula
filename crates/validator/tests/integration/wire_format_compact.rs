//! Scenario: wire format is externally-tagged tuple-compact — one entry
//! per outer variant proves the encoding contract.

use nebula_validator::{Predicate, Rule};
use serde_json::json;

fn golden(rule: Rule, expected: serde_json::Value) {
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, expected, "encode mismatch for {rule:?}");
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule, "roundtrip mismatch for {rule:?}");
}

#[test]
fn golden_min_length() {
    golden(Rule::min_length(3), json!({"min_length": 3}));
}

#[test]
fn golden_max_length() {
    golden(Rule::max_length(20), json!({"max_length": 20}));
}

#[test]
fn golden_pattern() {
    golden(Rule::pattern("^[a-z]+$"), json!({"pattern": "^[a-z]+$"}));
}

#[test]
fn golden_min_int() {
    golden(Rule::min_value(10), json!({"min": 10}));
}

#[test]
fn golden_one_of() {
    golden(Rule::one_of(["a", "b"]), json!({"one_of": ["a", "b"]}));
}

#[test]
fn golden_email_unit() {
    golden(Rule::email(), json!("email"));
}

#[test]
fn golden_url_unit() {
    golden(Rule::url(), json!("url"));
}

#[test]
fn golden_predicate_eq() {
    golden(
        Rule::predicate(Predicate::eq("status", json!("active")).unwrap()),
        json!({"eq": ["/status", "active"]}),
    );
}

#[test]
fn golden_predicate_is_true() {
    use nebula_validator::foundation::FieldPath;
    golden(
        Rule::predicate(Predicate::IsTrue(FieldPath::parse("enabled").unwrap())),
        json!({"is_true": "/enabled"}),
    );
}

#[test]
fn golden_logic_all() {
    golden(
        Rule::all([Rule::min_length(3), Rule::email()]),
        json!({"all": [{"min_length": 3}, "email"]}),
    );
}

#[test]
fn golden_logic_not() {
    golden(Rule::not(Rule::email()), json!({"not": "email"}));
}

#[test]
fn golden_deferred_custom() {
    golden(
        Rule::custom("check_password()"),
        json!({"custom": "check_password()"}),
    );
}

#[test]
fn golden_deferred_unique_by() {
    golden(
        Rule::unique_by("name").unwrap(),
        json!({"unique_by": "/name"}),
    );
}

#[test]
fn golden_described() {
    golden(
        Rule::min_length(3).with_message("too short"),
        json!({"described": [{"min_length": 3}, "too short"]}),
    );
}

#[test]
fn golden_nested_described() {
    golden(
        Rule::email().with_message("bad").with_message("worse"),
        json!({"described": [{"described": ["email", "bad"]}, "worse"]}),
    );
}

#[test]
fn sample_compound_rule_is_compact() {
    let rules = vec![Rule::min_length(3), Rule::max_length(100), Rule::email()];
    let encoded = serde_json::to_string(&rules).unwrap();
    assert!(
        encoded.len() <= 60,
        "compound rule grew: {} chars — {}",
        encoded.len(),
        encoded
    );
    assert_eq!(encoded, r#"[{"min_length":3},{"max_length":100},"email"]"#);
}
