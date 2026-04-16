//! Scenario: validating an inbound JSON API payload with the declarative
//! [`Rule`] engine. The payload shape is a `serde_json::Value`, rules come
//! from configuration (so we use the JSON-serialisable `Rule` enum, not
//! the derive macro), and the engine produces a flat [`ValidationErrors`]
//! collection suitable for mapping to an HTTP 400 response body.

use nebula_validator::{ExecutionMode, Rule, validate_rules};
use serde_json::json;

use super::common::{assert_codes_exactly, assert_has_code, expect_errors};

/// A bundle of rules describing an incoming `POST /users` payload. Each
/// entry represents one field; the engine validates each independently.
fn username_rules() -> Vec<Rule> {
    vec![
        Rule::MinLength {
            min: 3,
            message: None,
        },
        Rule::MaxLength {
            max: 32,
            message: None,
        },
        Rule::Pattern {
            pattern: r"^[a-z0-9_]+$".into(),
            message: Some("lowercase letters, digits, underscore only".into()),
        },
    ]
}

fn age_rules() -> Vec<Rule> {
    vec![
        Rule::Min {
            min: serde_json::Number::from(18),
            message: None,
        },
        Rule::Max {
            max: serde_json::Number::from(120),
            message: None,
        },
    ]
}

#[test]
fn valid_payload_passes_static_mode() {
    assert!(
        validate_rules(
            &json!("alice_42"),
            &username_rules(),
            ExecutionMode::StaticOnly,
        )
        .is_ok()
    );
    assert!(validate_rules(&json!(30), &age_rules(), ExecutionMode::StaticOnly).is_ok());
}

#[test]
fn invalid_payload_accumulates_every_rule_failure() {
    // "A" fails min_length AND the pattern (uppercase letter).
    let errors = expect_errors(validate_rules(
        &json!("A"),
        &username_rules(),
        ExecutionMode::StaticOnly,
    ));
    assert_has_code(&errors, "min_length");
    assert_has_code(&errors, "invalid_format"); // Pattern -> invalid_format
    assert_eq!(
        errors.len(),
        2,
        "expected two independent failures, got: [{}]",
        super::common::error_code_list(&errors)
    );
}

#[test]
fn custom_message_overrides_default() {
    let errors = expect_errors(validate_rules(
        &json!("UPPER"),
        &username_rules(),
        ExecutionMode::StaticOnly,
    ));

    let pattern_err = errors
        .errors()
        .iter()
        .find(|e| e.code.as_ref() == "invalid_format")
        .expect("pattern failure");
    assert_eq!(
        pattern_err.message.as_ref(),
        "lowercase letters, digits, underscore only"
    );
}

#[test]
fn deferred_rules_are_skipped_in_static_mode() {
    let rules = vec![
        Rule::MinLength {
            min: 3,
            message: None,
        },
        Rule::Custom {
            expression: "sibling_match('email')".into(),
            message: None,
        },
        Rule::UniqueBy {
            key: "id".into(),
            message: None,
        },
    ];

    // Only MinLength runs; Custom and UniqueBy are deferred.
    assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly).is_ok());

    // In Deferred mode, the static rule is skipped and deferred ones
    // return Ok (they need a runtime evaluator).
    assert!(validate_rules(&json!("ab"), &rules, ExecutionMode::Deferred).is_ok());
}

#[test]
fn combinator_rules_short_circuit_on_any() {
    let rules = vec![Rule::Any {
        rules: vec![
            Rule::MinLength {
                min: 10,
                message: None,
            },
            Rule::MaxLength {
                max: 3,
                message: None,
            },
        ],
    }];

    // "hello" is neither >=10 nor <=3 — both alternatives fail.
    let errors = expect_errors(validate_rules(
        &json!("hello"),
        &rules,
        ExecutionMode::StaticOnly,
    ));
    assert_codes_exactly(&errors, &["any_failed"]);

    // "ab" satisfies MaxLength — the combinator passes overall.
    assert!(validate_rules(&json!("ab"), &rules, ExecutionMode::StaticOnly).is_ok());
}

#[test]
fn type_mismatch_passes_silently_by_design() {
    // A number input does not trigger string rules; this is the documented
    // "permissive" behaviour that keeps rules composable across fields
    // with varying types.
    let rules = username_rules();
    assert!(validate_rules(&json!(42), &rules, ExecutionMode::StaticOnly).is_ok());
}
