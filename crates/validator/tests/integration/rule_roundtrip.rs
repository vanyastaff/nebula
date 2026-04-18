//! Scenario: `Rule` serialization contract — new externally-tagged
//! tuple-compact wire format. Rules pulled from JSON config must
//! roundtrip losslessly and validate the same values post-roundtrip.

use nebula_validator::{Predicate, Rule};
use serde_json::json;

#[test]
fn value_rule_compact_wire_form() {
    let rule = Rule::min_length(3);
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, json!({"min_length": 3}));
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn unit_rule_bare_string_wire_form() {
    let rule = Rule::email();
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, json!("email"));
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn predicate_rule_tuple_wire_form() {
    let rule = Rule::predicate(Predicate::eq("status", json!("active")).unwrap());
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded, json!({"eq": ["/status", "active"]}));
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn combinator_wire_form() {
    let rule = Rule::all([Rule::min_length(3), Rule::max_length(20)]);
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(
        encoded,
        json!({"all": [{"min_length": 3}, {"max_length": 20}]})
    );
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn described_wire_form() {
    let rule = Rule::min_length(3).with_message("too short");
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(
        encoded,
        json!({"described": [{"min_length": 3}, "too short"]})
    );
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn roundtrip_preserves_validation_behavior() {
    let original = Rule::pattern(r"^[a-z]+$");
    let decoded: Rule = serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();

    for probe in [json!("hello"), json!("Bad1"), json!(42), json!(null)] {
        let a = nebula_validator::foundation::Validate::validate(&original, &probe).is_ok();
        let b = nebula_validator::foundation::Validate::validate(&decoded, &probe).is_ok();
        assert_eq!(a, b, "rules disagree on {probe:?}");
    }
}

#[test]
fn described_roundtrip_with_template() {
    let rule = Rule::min_length(5).with_message("got {value}, need {min}");
    let decoded: Rule = serde_json::from_value(serde_json::to_value(&rule).unwrap()).unwrap();
    let err = nebula_validator::foundation::Validate::validate(&decoded, &json!("hi")).unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("got \"hi\", need 5"), "got: {rendered}");
}
