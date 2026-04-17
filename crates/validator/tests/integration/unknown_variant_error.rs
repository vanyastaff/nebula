//! Scenario: malformed JSON produces a descriptive error via our manual
//! Deserialize impl — not serde's generic "data did not match any variant".

use nebula_validator::Rule;
use serde_json::json;

#[test]
fn unknown_rule_name_lists_alternatives() {
    // "mn_length" is not a known typo so `typos` won't flag it; the
    // deserializer error message still suggests "min_length" from Known rules.
    let result: Result<Rule, _> = serde_json::from_value(json!({"mn_length": 3}));
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("unknown rule"), "got: {msg}");
    assert!(msg.contains("min_length"), "got: {msg}");
    assert!(msg.contains("Known rules:"), "got: {msg}");
}

#[test]
fn empty_object_rejected() {
    let result: Result<Rule, _> = serde_json::from_value(json!({}));
    assert!(result.is_err());
}

#[test]
fn unknown_unit_string_rejected() {
    let result: Result<Rule, _> = serde_json::from_value(json!("not_a_rule"));
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown"));
}

#[test]
fn multi_key_object_rejected() {
    let result: Result<Rule, _> =
        serde_json::from_value(json!({"min_length": 3, "max_length": 10}));
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("exactly one key") || err.to_string().contains("extra key"),
        "got: {err}"
    );
}
