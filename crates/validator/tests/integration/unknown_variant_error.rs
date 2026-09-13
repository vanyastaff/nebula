//! Scenario: malformed JSON produces stable diagnostics without echoing
//! attacker-controlled rule keys.

use nebula_validator::Rule;
use serde_json::json;

#[test]
fn unknown_rule_name_is_redacted() {
    const SECRET_KEY: &str = "unknown_rule_key_secret_86d1f2";
    let result: Result<Rule, _> = serde_json::from_value(json!({(SECRET_KEY): 3}));
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert_eq!(msg, "unknown rule key");
    assert!(!msg.contains(SECRET_KEY));
}

#[test]
fn empty_object_rejected() {
    let result: Result<Rule, _> = serde_json::from_value(json!({}));
    assert!(result.is_err());
}

#[test]
fn unknown_unit_string_rejected() {
    const SECRET_NAME: &str = "unknown_unit_secret_222c51";
    let result: Result<Rule, _> = serde_json::from_value(json!(SECRET_NAME));
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown"));
    assert!(!err.to_string().contains(SECRET_NAME));
}

#[test]
fn multi_key_object_rejected() {
    const SECRET_KEY: &str = "zz_extra_rule_key_secret_304398";
    let result: Result<Rule, _> =
        serde_json::from_value(json!({"min_length": 3, (SECRET_KEY): 10}));
    let err = result.unwrap_err();
    assert!(err.to_string().contains("exactly one key"), "got: {err}");
    assert!(!err.to_string().contains(SECRET_KEY));
}

#[test]
fn unit_variant_in_map_form_consumes_value() {
    // Serde's MapAccess contract: every next_key() must be paired with a
    // next_value() call. Previously the email/url arms skipped next_value,
    // which works by accident with serde_json but violates the contract for
    // other data formats (RON, MessagePack, etc.). Consume the unit payload
    // while rejecting configuration the unit rule cannot enforce.
    let r: Result<Rule, _> = serde_json::from_value(json!({"email": null}));
    assert_eq!(r.unwrap(), Rule::email());

    let r: Result<Rule, _> = serde_json::from_value(json!({"url": null}));
    assert_eq!(r.unwrap(), Rule::url());

    let r: Result<Rule, _> = serde_json::from_value(json!({"url": {"unrelated": 42}}));
    assert!(
        r.is_err(),
        "map-form url must reject ignored configuration: {r:?}"
    );

    // And multi-key still rejects even when the unit variant is first.
    let r: Result<Rule, _> = serde_json::from_value(json!({"email": null, "url": null}));
    assert!(
        r.is_err(),
        "multi-key object with unit variant first should reject"
    );
}
