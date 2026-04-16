//! Scenario: `Rule` serialization contract — rules pulled from JSON
//! config must roundtrip losslessly, cover every category, and validate
//! the same values regardless of whether they went through serde.

use nebula_validator::Rule;
use serde_json::json;

#[test]
fn value_rule_roundtrip() {
    let rule = Rule::MinLength {
        min: 3,
        message: Some("too short".into()),
    };
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded["rule"], "min_length");
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn predicate_rule_roundtrip() {
    let rule = Rule::Eq {
        field: "status".into(),
        value: json!("active"),
    };
    let encoded = serde_json::to_value(&rule).unwrap();
    assert_eq!(encoded["rule"], "eq");
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn combinator_rule_roundtrip() {
    let rule = Rule::All {
        rules: vec![
            Rule::MinLength {
                min: 3,
                message: None,
            },
            Rule::Eq {
                field: "kind".into(),
                value: json!("user"),
            },
        ],
    };
    let encoded = serde_json::to_value(&rule).unwrap();
    let decoded: Rule = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, rule);
}

#[test]
fn roundtrip_preserves_validation_behavior() {
    let original = Rule::Pattern {
        pattern: r"^[a-z]+$".into(),
        message: None,
    };
    let decoded: Rule = serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();

    // Both rules must agree on every probe input.
    for probe in [json!("hello"), json!("Bad1"), json!(42), json!(null)] {
        assert_eq!(
            original.validate_value(&probe).is_ok(),
            decoded.validate_value(&probe).is_ok(),
            "roundtripped rule disagrees on input {probe:?}",
        );
    }
}

#[test]
fn message_override_survives_roundtrip() {
    let rule = Rule::MinLength {
        min: 5,
        message: Some("please enter at least 5 characters".into()),
    };
    let decoded: Rule = serde_json::from_value(serde_json::to_value(&rule).unwrap()).unwrap();

    let err = decoded.validate_value(&json!("ab")).unwrap_err();
    assert_eq!(err.message.as_ref(), "please enter at least 5 characters");
}

#[test]
fn classification_round_trip_is_stable() {
    let rules: Vec<Rule> = vec![
        Rule::MinLength {
            min: 1,
            message: None,
        },
        Rule::Eq {
            field: "x".into(),
            value: json!(1),
        },
        Rule::Custom {
            expression: "y > 0".into(),
            message: None,
        },
    ];

    for rule in &rules {
        let decoded: Rule = serde_json::from_value(serde_json::to_value(rule).unwrap()).unwrap();
        assert_eq!(decoded.is_value_rule(), rule.is_value_rule());
        assert_eq!(decoded.is_predicate(), rule.is_predicate());
        assert_eq!(decoded.is_deferred(), rule.is_deferred());
    }
}

#[test]
fn unknown_rule_tag_is_rejected() {
    let bad = json!({ "rule": "teleport", "to": "mars" });
    assert!(
        serde_json::from_value::<Rule>(bad).is_err(),
        "deserialization must reject unknown rule tags"
    );
}
