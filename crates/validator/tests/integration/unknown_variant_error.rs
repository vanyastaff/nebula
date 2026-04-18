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

#[test]
fn unit_variant_in_map_form_consumes_value() {
    // Serde's MapAccess contract: every next_key() must be paired with a
    // next_value() call. Previously the email/url arms skipped next_value,
    // which works by accident with serde_json but violates the contract for
    // other data formats (RON, MessagePack, etc.). Now guarded with
    // IgnoredAny so any payload shape is accepted leniently.
    let r: Result<Rule, _> = serde_json::from_value(json!({"email": null}));
    assert!(
        r.is_ok(),
        "map-form email with null value should deserialize: {r:?}"
    );

    let r: Result<Rule, _> = serde_json::from_value(json!({"url": {"unrelated": 42}}));
    assert!(
        r.is_ok(),
        "map-form url with complex value should deserialize: {r:?}"
    );

    // And multi-key still rejects even when the unit variant is first.
    let r: Result<Rule, _> = serde_json::from_value(json!({"email": null, "url": null}));
    assert!(
        r.is_err(),
        "multi-key object with unit variant first should reject"
    );
}
