//! Serialization roundtrip coverage.
//!
//! NOTE: This module is being rewritten in Task 13 of the
//! nebula-validator Rule refactor for the new externally-tagged
//! tuple-compact wire format (`{"min_length": 3}`, `{"eq": ["/path", v]}`).
//! Placeholder until that lands.

use nebula_validator::Rule;

#[test]
fn placeholder_value_rule_roundtrip() {
    let rule = Rule::min_length(3);
    let encoded = serde_json::to_value(&rule).unwrap();
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}
