//! Integration tests for the `Rule` enum: value validation, context
//! predicates, combinators, serde roundtrip, and edge cases.

use std::collections::HashMap;

use serde_json::json;

use super::*;

// ── Value validation ────────────────────────────────────────────────

#[test]
fn min_length_passes() {
    let rule = Rule::MinLength {
        min: 3,
        message: None,
    };
    assert!(rule.validate_value(&json!("alice")).is_ok());
}

#[test]
fn min_length_fails() {
    let rule = Rule::MinLength {
        min: 3,
        message: None,
    };
    let err = rule.validate_value(&json!("ab")).unwrap_err();
    assert_eq!(err.code.as_ref(), "min_length");
}

#[test]
fn min_length_custom_message() {
    let rule = Rule::MinLength {
        min: 5,
        message: Some("too short!".into()),
    };
    let err = rule.validate_value(&json!("ab")).unwrap_err();
    assert_eq!(err.message.as_ref(), "too short!");
}

#[test]
fn max_length_passes() {
    let rule = Rule::MaxLength {
        max: 10,
        message: None,
    };
    assert!(rule.validate_value(&json!("hello")).is_ok());
}

#[test]
fn max_length_fails() {
    let rule = Rule::MaxLength {
        max: 3,
        message: None,
    };
    assert!(rule.validate_value(&json!("hello")).is_err());
}

#[test]
fn pattern_passes() {
    let rule = Rule::Pattern {
        pattern: "^[a-z]+$".into(),
        message: None,
    };
    assert!(rule.validate_value(&json!("hello")).is_ok());
}

#[test]
fn pattern_fails() {
    let rule = Rule::Pattern {
        pattern: "^[a-z]+$".into(),
        message: None,
    };
    assert!(rule.validate_value(&json!("Hello123")).is_err());
}

#[test]
fn min_numeric_passes() {
    let rule = Rule::Min {
        min: serde_json::Number::from(5),
        message: None,
    };
    assert!(rule.validate_value(&json!(10)).is_ok());
}

#[test]
fn min_numeric_fails() {
    let rule = Rule::Min {
        min: serde_json::Number::from(5),
        message: None,
    };
    assert!(rule.validate_value(&json!(3)).is_err());
}

#[test]
fn max_numeric_passes() {
    let rule = Rule::Max {
        max: serde_json::Number::from(100),
        message: None,
    };
    assert!(rule.validate_value(&json!(50)).is_ok());
}

#[test]
fn max_numeric_fails() {
    let rule = Rule::Max {
        max: serde_json::Number::from(10),
        message: None,
    };
    assert!(rule.validate_value(&json!(20)).is_err());
}

#[test]
fn one_of_passes() {
    let rule = Rule::OneOf {
        values: vec![json!("a"), json!("b"), json!("c")],
        message: None,
    };
    assert!(rule.validate_value(&json!("b")).is_ok());
}

#[test]
fn one_of_fails() {
    let rule = Rule::OneOf {
        values: vec![json!("a"), json!("b")],
        message: None,
    };
    assert!(rule.validate_value(&json!("x")).is_err());
}

#[test]
fn min_items_passes() {
    let rule = Rule::MinItems {
        min: 2,
        message: None,
    };
    assert!(rule.validate_value(&json!([1, 2, 3])).is_ok());
}

#[test]
fn min_items_fails() {
    let rule = Rule::MinItems {
        min: 3,
        message: None,
    };
    assert!(rule.validate_value(&json!([1])).is_err());
}

#[test]
fn max_items_passes() {
    let rule = Rule::MaxItems {
        max: 5,
        message: None,
    };
    assert!(rule.validate_value(&json!([1, 2])).is_ok());
}

#[test]
fn max_items_fails() {
    let rule = Rule::MaxItems {
        max: 2,
        message: None,
    };
    assert!(rule.validate_value(&json!([1, 2, 3])).is_err());
}

#[test]
fn deferred_rules_skip() {
    let rule = Rule::UniqueBy {
        key: "id".into(),
        message: None,
    };
    assert!(rule.validate_value(&json!([1, 1])).is_ok());
    assert!(rule.is_deferred());
}

#[test]
fn non_matching_type_passes_silently() {
    // MinLength on a number → skip, no error
    let rule = Rule::MinLength {
        min: 3,
        message: None,
    };
    assert!(rule.validate_value(&json!(42)).is_ok());
}

// ── Context predicates ──────────────────────────────────────────────

fn values(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn eq_predicate() {
    let rule = Rule::Eq {
        field: "status".into(),
        value: json!("active"),
    };
    assert!(rule.evaluate(&values(&[("status", json!("active"))])));
    assert!(!rule.evaluate(&values(&[("status", json!("inactive"))])));
}

#[test]
fn ne_predicate() {
    let rule = Rule::Ne {
        field: "status".into(),
        value: json!("deleted"),
    };
    assert!(rule.evaluate(&values(&[("status", json!("active"))])));
    assert!(!rule.evaluate(&values(&[("status", json!("deleted"))])));
}

#[test]
fn gt_predicate() {
    let rule = Rule::Gt {
        field: "age".into(),
        value: serde_json::Number::from(18),
    };
    assert!(rule.evaluate(&values(&[("age", json!(20))])));
    assert!(!rule.evaluate(&values(&[("age", json!(18))])));
}

#[test]
fn gte_predicate() {
    let rule = Rule::Gte {
        field: "age".into(),
        value: serde_json::Number::from(18),
    };
    assert!(rule.evaluate(&values(&[("age", json!(20))])));
    assert!(rule.evaluate(&values(&[("age", json!(18))])));
    assert!(!rule.evaluate(&values(&[("age", json!(17))])));
}

#[test]
fn lt_predicate() {
    let rule = Rule::Lt {
        field: "count".into(),
        value: serde_json::Number::from(10),
    };
    assert!(rule.evaluate(&values(&[("count", json!(5))])));
    assert!(!rule.evaluate(&values(&[("count", json!(10))])));
    assert!(!rule.evaluate(&values(&[("count", json!(15))])));
}

#[test]
fn lte_predicate() {
    let rule = Rule::Lte {
        field: "count".into(),
        value: serde_json::Number::from(10),
    };
    assert!(rule.evaluate(&values(&[("count", json!(5))])));
    assert!(rule.evaluate(&values(&[("count", json!(10))])));
    assert!(!rule.evaluate(&values(&[("count", json!(11))])));
}

#[test]
fn is_true_predicate() {
    let rule = Rule::IsTrue {
        field: "enabled".into(),
    };
    assert!(rule.evaluate(&values(&[("enabled", json!(true))])));
    assert!(!rule.evaluate(&values(&[("enabled", json!(false))])));
}

#[test]
fn is_false_predicate() {
    let rule = Rule::IsFalse {
        field: "disabled".into(),
    };
    assert!(rule.evaluate(&values(&[("disabled", json!(false))])));
    assert!(!rule.evaluate(&values(&[("disabled", json!(true))])));
}

#[test]
fn set_predicate() {
    let rule = Rule::Set {
        field: "name".into(),
    };
    assert!(rule.evaluate(&values(&[("name", json!("Alice"))])));
    assert!(!rule.evaluate(&values(&[("name", json!(""))])));
    assert!(!rule.evaluate(&values(&[("name", json!(null))])));
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn empty_predicate() {
    let rule = Rule::Empty {
        field: "name".into(),
    };
    assert!(rule.evaluate(&values(&[])));
    assert!(rule.evaluate(&values(&[("name", json!(null))])));
    assert!(rule.evaluate(&values(&[("name", json!(""))])));
    assert!(!rule.evaluate(&values(&[("name", json!("Alice"))])));
}

#[test]
fn contains_string_predicate() {
    let rule = Rule::Contains {
        field: "tags".into(),
        value: json!("rust"),
    };
    assert!(rule.evaluate(&values(&[("tags", json!("I love rust"))])));
    assert!(!rule.evaluate(&values(&[("tags", json!("I love go"))])));
}

#[test]
fn contains_array_predicate() {
    let rule = Rule::Contains {
        field: "tags".into(),
        value: json!("rust"),
    };
    assert!(rule.evaluate(&values(&[("tags", json!(["rust", "go"]))])));
    assert!(!rule.evaluate(&values(&[("tags", json!(["python"]))])));
}

#[test]
fn in_predicate() {
    let rule = Rule::In {
        field: "role".into(),
        values: vec![json!("admin"), json!("editor")],
    };
    assert!(rule.evaluate(&values(&[("role", json!("admin"))])));
    assert!(!rule.evaluate(&values(&[("role", json!("viewer"))])));
}

#[test]
fn matches_predicate() {
    let rule = Rule::Matches {
        field: "email".into(),
        pattern: r"^[^@]+@[^@]+$".into(),
    };
    assert!(rule.evaluate(&values(&[("email", json!("a@b.com"))])));
    assert!(!rule.evaluate(&values(&[("email", json!("invalid"))])));
}

// ── Logical combinators ─────────────────────────────────────────────

#[test]
fn all_combinator() {
    let rule = Rule::All {
        rules: vec![
            Rule::MinLength {
                min: 3,
                message: None,
            },
            Rule::MaxLength {
                max: 10,
                message: None,
            },
        ],
    };
    assert!(rule.validate_value(&json!("hello")).is_ok());
    assert!(rule.validate_value(&json!("ab")).is_err());
    assert!(rule.validate_value(&json!("hello world!")).is_err());
}

#[test]
fn any_combinator() {
    let rule = Rule::Any {
        rules: vec![
            Rule::Eq {
                field: "a".into(),
                value: json!(1),
            },
            Rule::Eq {
                field: "b".into(),
                value: json!(2),
            },
        ],
    };
    assert!(rule.evaluate(&values(&[("a", json!(1))])));
    assert!(rule.evaluate(&values(&[("b", json!(2))])));
    assert!(!rule.evaluate(&values(&[("a", json!(9)), ("b", json!(9))])));
}

#[test]
fn not_combinator_predicate() {
    let rule = Rule::Not {
        inner: Box::new(Rule::Eq {
            field: "x".into(),
            value: json!(0),
        }),
    };
    assert!(rule.evaluate(&values(&[("x", json!(1))])));
    assert!(!rule.evaluate(&values(&[("x", json!(0))])));
}

#[test]
fn not_combinator_value() {
    let rule = Rule::Not {
        inner: Box::new(Rule::MinLength {
            min: 5,
            message: None,
        }),
    };
    assert!(rule.validate_value(&json!("ab")).is_ok()); // MinLength fails → Not passes
    assert!(rule.validate_value(&json!("hello")).is_err()); // MinLength passes → Not fails
}

// ── Serde roundtrip ─────────────────────────────────────────────────

#[test]
fn serde_roundtrip_value_rule() {
    let rule = Rule::MinLength {
        min: 3,
        message: Some("too short".into()),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "min_length");
    assert_eq!(json["min"], 3);
    assert_eq!(json["message"], "too short");

    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_predicate() {
    let rule = Rule::Eq {
        field: "status".into(),
        value: json!("active"),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "eq");

    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_combinator() {
    let rule = Rule::All {
        rules: vec![
            Rule::MinLength {
                min: 3,
                message: None,
            },
            Rule::Eq {
                field: "x".into(),
                value: json!(1),
            },
        ],
    };
    let json = serde_json::to_value(&rule).unwrap();
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

// ── Classification ──────────────────────────────────────────────────

#[test]
fn classification() {
    assert!(
        Rule::MinLength {
            min: 1,
            message: None
        }
        .is_value_rule()
    );
    assert!(
        !Rule::MinLength {
            min: 1,
            message: None
        }
        .is_predicate()
    );
    assert!(
        !Rule::MinLength {
            min: 1,
            message: None
        }
        .is_deferred()
    );

    assert!(
        Rule::Eq {
            field: "x".into(),
            value: json!(1)
        }
        .is_predicate()
    );
    assert!(
        !Rule::Eq {
            field: "x".into(),
            value: json!(1)
        }
        .is_value_rule()
    );

    assert!(
        Rule::UniqueBy {
            key: "id".into(),
            message: None
        }
        .is_deferred()
    );
    assert!(
        Rule::Custom {
            expression: "true".into(),
            message: None
        }
        .is_deferred()
    );
}

// ── Missing field edge cases ────────────────────────────────────────

#[test]
fn eq_missing_field_is_false() {
    let rule = Rule::Eq {
        field: "x".into(),
        value: json!(1),
    };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn ne_missing_field_is_true() {
    let rule = Rule::Ne {
        field: "x".into(),
        value: json!(1),
    };
    // Missing field can't equal value, so Ne is true.
    assert!(rule.evaluate(&values(&[])));
}

#[test]
fn gt_missing_field_is_false() {
    let rule = Rule::Gt {
        field: "x".into(),
        value: serde_json::Number::from(0),
    };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn gt_non_numeric_field_is_false() {
    let rule = Rule::Gt {
        field: "x".into(),
        value: serde_json::Number::from(0),
    };
    assert!(!rule.evaluate(&values(&[("x", json!("text"))])));
}

#[test]
fn gte_missing_field_is_false() {
    let rule = Rule::Gte {
        field: "x".into(),
        value: serde_json::Number::from(0),
    };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn lt_missing_field_is_false() {
    let rule = Rule::Lt {
        field: "x".into(),
        value: serde_json::Number::from(0),
    };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn lte_missing_field_is_false() {
    let rule = Rule::Lte {
        field: "x".into(),
        value: serde_json::Number::from(0),
    };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn is_true_missing_field_is_false() {
    let rule = Rule::IsTrue { field: "x".into() };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn is_true_non_bool_is_false() {
    let rule = Rule::IsTrue { field: "x".into() };
    assert!(!rule.evaluate(&values(&[("x", json!(1))])));
}

#[test]
fn is_false_missing_field_is_false() {
    let rule = Rule::IsFalse { field: "x".into() };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn is_false_non_bool_is_false() {
    let rule = Rule::IsFalse { field: "x".into() };
    assert!(!rule.evaluate(&values(&[("x", json!(0))])));
}

#[test]
fn set_with_number_is_true() {
    let rule = Rule::Set { field: "x".into() };
    assert!(rule.evaluate(&values(&[("x", json!(0))])));
}

#[test]
fn set_with_empty_array_is_false() {
    let rule = Rule::Set { field: "x".into() };
    assert!(!rule.evaluate(&values(&[("x", json!([]))])));
}

#[test]
fn empty_with_empty_array_is_true() {
    let rule = Rule::Empty { field: "x".into() };
    assert!(rule.evaluate(&values(&[("x", json!([]))])));
}

#[test]
fn empty_with_non_empty_array_is_false() {
    let rule = Rule::Empty { field: "x".into() };
    assert!(!rule.evaluate(&values(&[("x", json!([1]))])));
}

#[test]
fn empty_with_number_is_false() {
    let rule = Rule::Empty { field: "x".into() };
    assert!(!rule.evaluate(&values(&[("x", json!(0))])));
}

#[test]
fn contains_non_string_non_array_is_false() {
    let rule = Rule::Contains {
        field: "x".into(),
        value: json!(1),
    };
    assert!(!rule.evaluate(&values(&[("x", json!(42))])));
}

#[test]
fn contains_missing_field_is_false() {
    let rule = Rule::Contains {
        field: "x".into(),
        value: json!("a"),
    };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
fn matches_missing_field_is_false() {
    let rule = Rule::Matches {
        field: "x".into(),
        pattern: ".*".into(),
    };
    assert!(!rule.evaluate(&values(&[])));
}

#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "invalid regex pattern"))]
fn matches_invalid_regex_is_false() {
    let rule = Rule::Matches {
        field: "x".into(),
        pattern: "[invalid".into(),
    };
    // In release mode: silently returns false (degenerate but non-panicking).
    // In debug mode: debug_assert fires to alert the developer.
    assert!(!rule.evaluate(&values(&[("x", json!("anything"))])));
}

#[test]
fn in_missing_field_is_false() {
    let rule = Rule::In {
        field: "x".into(),
        values: vec![json!(1), json!(2)],
    };
    assert!(!rule.evaluate(&values(&[])));
}

// ── Numeric predicate with floats ───────────────────────────────────

#[expect(
    clippy::approx_constant,
    reason = "3.14 is a representative float literal, not an approximation of π"
)]
#[test]
fn gt_float_comparison() {
    let rule = Rule::Gt {
        field: "val".into(),
        value: serde_json::Number::from_f64(3.14).unwrap(),
    };
    assert!(rule.evaluate(&values(&[("val", json!(3.15))])));
    assert!(!rule.evaluate(&values(&[("val", json!(3.14))])));
    assert!(!rule.evaluate(&values(&[("val", json!(3.13))])));
}

// ── Value rules return true in evaluate ─────────────────────────────

#[test]
fn value_rule_evaluate_returns_true() {
    let rule = Rule::MinLength {
        min: 100,
        message: None,
    };
    // Value rules are vacuously true when used as predicates.
    assert!(rule.evaluate(&values(&[])));
}

#[test]
fn deferred_rule_evaluate_returns_true() {
    let rule = Rule::Custom {
        expression: "false".into(),
        message: None,
    };
    assert!(rule.evaluate(&values(&[])));
}

// ── Predicates return Ok in validate_value ──────────────────────────

#[test]
fn predicate_validate_value_returns_ok() {
    let rule = Rule::Eq {
        field: "x".into(),
        value: json!(1),
    };
    assert!(rule.validate_value(&json!("anything")).is_ok());
}

// ── Value validation edge cases ─────────────────────────────────────

#[test]
fn pattern_invalid_regex_returns_error() {
    let rule = Rule::Pattern {
        pattern: "[invalid".into(),
        message: None,
    };
    let err = rule.validate_value(&json!("test")).unwrap_err();
    assert_eq!(err.code.as_ref(), "invalid_pattern");
}

#[test]
fn try_pattern_validates_regex_upfront() {
    assert!(Rule::try_pattern(r"^\d+$").is_some());
    assert!(Rule::try_pattern(r"[invalid").is_none());
}

#[test]
fn try_pattern_with_message() {
    let rule = Rule::try_pattern(r"^\d+$")
        .unwrap()
        .with_message("digits only");
    let err = rule.validate_value(&json!("abc")).unwrap_err();
    assert_eq!(err.message.as_ref(), "digits only");
}

#[expect(
    clippy::approx_constant,
    reason = "3.14 is a representative float literal, not an approximation of π"
)]
#[test]
fn min_float_boundary() {
    let rule = Rule::Min {
        min: serde_json::Number::from_f64(3.14).unwrap(),
        message: None,
    };
    assert!(rule.validate_value(&json!(3.14)).is_ok());
    assert!(rule.validate_value(&json!(3.15)).is_ok());
    assert!(rule.validate_value(&json!(3.13)).is_err());
}

#[test]
fn max_float_boundary() {
    let rule = Rule::Max {
        max: serde_json::Number::from_f64(9.99).unwrap(),
        message: None,
    };
    assert!(rule.validate_value(&json!(9.99)).is_ok());
    assert!(rule.validate_value(&json!(9.98)).is_ok());
    assert!(rule.validate_value(&json!(10.0)).is_err());
}

#[test]
fn min_on_non_number_is_ok() {
    let rule = Rule::Min {
        min: serde_json::Number::from(5),
        message: None,
    };
    assert!(rule.validate_value(&json!("text")).is_ok());
}

#[test]
fn max_on_non_number_is_ok() {
    let rule = Rule::Max {
        max: serde_json::Number::from(5),
        message: None,
    };
    assert!(rule.validate_value(&json!(null)).is_ok());
}

#[test]
fn one_of_with_mixed_types() {
    let rule = Rule::OneOf {
        values: vec![json!(1), json!("yes"), json!(true)],
        message: None,
    };
    assert!(rule.validate_value(&json!(1)).is_ok());
    assert!(rule.validate_value(&json!("yes")).is_ok());
    assert!(rule.validate_value(&json!(true)).is_ok());
    assert!(rule.validate_value(&json!(false)).is_err());
}

#[test]
fn one_of_custom_message() {
    let rule = Rule::OneOf {
        values: vec![json!("a")],
        message: Some("pick something valid".into()),
    };
    let err = rule.validate_value(&json!("z")).unwrap_err();
    assert_eq!(err.message.as_ref(), "pick something valid");
}

#[test]
fn min_items_on_non_array_is_ok() {
    let rule = Rule::MinItems {
        min: 5,
        message: None,
    };
    assert!(rule.validate_value(&json!("not an array")).is_ok());
}

#[test]
fn max_items_on_non_array_is_ok() {
    let rule = Rule::MaxItems {
        max: 1,
        message: None,
    };
    assert!(rule.validate_value(&json!(42)).is_ok());
}

#[test]
fn min_length_on_null_is_ok() {
    let rule = Rule::MinLength {
        min: 3,
        message: None,
    };
    assert!(rule.validate_value(&json!(null)).is_ok());
}

#[test]
fn max_length_custom_message() {
    let rule = Rule::MaxLength {
        max: 3,
        message: Some("way too long".into()),
    };
    let err = rule.validate_value(&json!("hello")).unwrap_err();
    assert_eq!(err.message.as_ref(), "way too long");
}

#[test]
fn pattern_custom_message() {
    let rule = Rule::Pattern {
        pattern: "^[0-9]+$".into(),
        message: Some("digits only!".into()),
    };
    let err = rule.validate_value(&json!("abc")).unwrap_err();
    assert_eq!(err.message.as_ref(), "digits only!");
}

#[test]
fn min_items_exact_boundary() {
    let rule = Rule::MinItems {
        min: 2,
        message: None,
    };
    assert!(rule.validate_value(&json!([1, 2])).is_ok());
    assert!(rule.validate_value(&json!([1])).is_err());
}

#[test]
fn max_items_exact_boundary() {
    let rule = Rule::MaxItems {
        max: 2,
        message: None,
    };
    assert!(rule.validate_value(&json!([1, 2])).is_ok());
    assert!(rule.validate_value(&json!([1, 2, 3])).is_err());
}

#[test]
fn validate_null_value_passes_all_value_rules() {
    let rules = vec![
        Rule::MinLength {
            min: 1,
            message: None,
        },
        Rule::MaxLength {
            max: 1,
            message: None,
        },
        Rule::Pattern {
            pattern: "^x$".into(),
            message: None,
        },
        Rule::Min {
            min: serde_json::Number::from(1),
            message: None,
        },
        Rule::Max {
            max: serde_json::Number::from(1),
            message: None,
        },
        Rule::MinItems {
            min: 1,
            message: None,
        },
        Rule::MaxItems {
            max: 1,
            message: None,
        },
        Rule::Email { message: None },
        Rule::Url { message: None },
    ];
    for rule in &rules {
        assert!(
            rule.validate_value(&json!(null)).is_ok(),
            "rule {:?} should pass on null",
            rule
        );
    }
}

// ── Combinator edge cases ───────────────────────────────────────────

#[test]
fn all_empty_rules_passes() {
    let rule = Rule::All { rules: vec![] };
    assert!(rule.validate_value(&json!("anything")).is_ok());
}

#[test]
fn any_empty_rules_passes() {
    let rule = Rule::Any { rules: vec![] };
    assert!(rule.validate_value(&json!("anything")).is_ok());
}

#[test]
fn all_with_single_failing_rule() {
    let rule = Rule::All {
        rules: vec![
            Rule::MinLength {
                min: 1,
                message: None,
            },
            Rule::MaxLength {
                max: 3,
                message: None,
            },
            Rule::Pattern {
                pattern: "^[0-9]+$".into(),
                message: None,
            },
        ],
    };
    // "ab" passes MinLength and MaxLength but fails Pattern
    assert!(rule.validate_value(&json!("ab")).is_err());
}

#[test]
fn any_first_passes() {
    let rule = Rule::Any {
        rules: vec![
            Rule::MinLength {
                min: 1,
                message: None,
            },
            Rule::MinLength {
                min: 100,
                message: None,
            },
        ],
    };
    assert!(rule.validate_value(&json!("hello")).is_ok());
}

#[test]
fn any_last_passes() {
    let rule = Rule::Any {
        rules: vec![
            Rule::MinLength {
                min: 100,
                message: None,
            },
            Rule::MinLength {
                min: 1,
                message: None,
            },
        ],
    };
    assert!(rule.validate_value(&json!("hello")).is_ok());
}

#[test]
fn nested_combinators() {
    // All(Any(MinLength(10), MaxLength(3)), Pattern(^[a-z]+$))
    let rule = Rule::All {
        rules: vec![
            Rule::Any {
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
            },
            Rule::Pattern {
                pattern: "^[a-z]+$".into(),
                message: None,
            },
        ],
    };
    // "ab" → Any(MinLength(10) fails, MaxLength(3) passes) → ok; Pattern passes → ok
    assert!(rule.validate_value(&json!("ab")).is_ok());
    // "AB" → Any passes, but Pattern fails
    assert!(rule.validate_value(&json!("AB")).is_err());
    // "abcde" → Any(MinLength fails, MaxLength fails) → fails
    assert!(rule.validate_value(&json!("abcde")).is_err());
}

#[test]
fn not_with_not_double_negation() {
    let rule = Rule::Not {
        inner: Box::new(Rule::Not {
            inner: Box::new(Rule::MinLength {
                min: 3,
                message: None,
            }),
        }),
    };
    // Double negation: Not(Not(MinLength(3))) == MinLength(3)
    assert!(rule.validate_value(&json!("hello")).is_ok());
    assert!(rule.validate_value(&json!("ab")).is_err());
}

#[test]
fn all_evaluate_with_predicates() {
    let rule = Rule::All {
        rules: vec![
            Rule::Eq {
                field: "a".into(),
                value: json!(1),
            },
            Rule::Set { field: "b".into() },
        ],
    };
    assert!(rule.evaluate(&values(&[("a", json!(1)), ("b", json!("x"))])));
    assert!(!rule.evaluate(&values(&[("a", json!(1))])));
    assert!(!rule.evaluate(&values(&[("b", json!("x"))])));
}

#[test]
fn any_evaluate_with_predicates() {
    let rule = Rule::Any {
        rules: vec![
            Rule::Eq {
                field: "a".into(),
                value: json!(1),
            },
            Rule::Set { field: "b".into() },
        ],
    };
    assert!(rule.evaluate(&values(&[("a", json!(1))])));
    assert!(rule.evaluate(&values(&[("b", json!("x"))])));
    assert!(!rule.evaluate(&values(&[])));
}

// ── Serde all predicate variants ────────────────────────────────────

#[test]
fn serde_roundtrip_gt() {
    let rule = Rule::Gt {
        field: "x".into(),
        value: serde_json::Number::from(5),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "gt");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_gte() {
    let rule = Rule::Gte {
        field: "x".into(),
        value: serde_json::Number::from(5),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "gte");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_lt() {
    let rule = Rule::Lt {
        field: "x".into(),
        value: serde_json::Number::from(5),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "lt");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_lte() {
    let rule = Rule::Lte {
        field: "x".into(),
        value: serde_json::Number::from(5),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "lte");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_is_true() {
    let rule = Rule::IsTrue { field: "x".into() };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "is_true");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_is_false() {
    let rule = Rule::IsFalse { field: "x".into() };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "is_false");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_set() {
    let rule = Rule::Set { field: "x".into() };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "set");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_empty() {
    let rule = Rule::Empty { field: "x".into() };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "empty");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_contains() {
    let rule = Rule::Contains {
        field: "tags".into(),
        value: json!("rust"),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "contains");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_matches() {
    let rule = Rule::Matches {
        field: "email".into(),
        pattern: r"^[^@]+@[^@]+$".into(),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "matches");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_in() {
    let rule = Rule::In {
        field: "role".into(),
        values: vec![json!("admin"), json!("editor")],
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "in");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_ne() {
    let rule = Rule::Ne {
        field: "status".into(),
        value: json!("deleted"),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "ne");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_not() {
    let rule = Rule::Not {
        inner: Box::new(Rule::Eq {
            field: "x".into(),
            value: json!(1),
        }),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "not");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_any() {
    let rule = Rule::Any {
        rules: vec![
            Rule::Eq {
                field: "a".into(),
                value: json!(1),
            },
            Rule::IsTrue { field: "b".into() },
        ],
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "any");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_unique_by() {
    let rule = Rule::UniqueBy {
        key: "id".into(),
        message: Some("must be unique".into()),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "unique_by");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_roundtrip_custom() {
    let rule = Rule::Custom {
        expression: "len(items) > 0".into(),
        message: None,
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(json["rule"], "custom");
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn serde_deserialize_from_json_string() {
    let json_str = r#"{"rule":"min_length","min":5}"#;
    let rule: Rule = serde_json::from_str(json_str).unwrap();
    assert_eq!(
        rule,
        Rule::MinLength {
            min: 5,
            message: None
        }
    );
}

#[test]
fn serde_deserialize_nested_combinator() {
    let json_str = r#"{
        "rule": "all",
        "rules": [
            {"rule": "min_length", "min": 3},
            {"rule": "not", "inner": {"rule": "eq", "field": "x", "value": 1}}
        ]
    }"#;
    let rule: Rule = serde_json::from_str(json_str).unwrap();
    match rule {
        Rule::All { rules } => assert_eq!(rules.len(), 2),
        other => panic!("expected All, got {other:?}"),
    }
}

// ── Classification completeness ─────────────────────────────────────

#[test]
fn all_combinators_are_neither_value_nor_predicate_nor_deferred() {
    let combinators = vec![
        Rule::All { rules: vec![] },
        Rule::Any { rules: vec![] },
        Rule::Not {
            inner: Box::new(Rule::IsTrue { field: "x".into() }),
        },
    ];
    for c in &combinators {
        assert!(!c.is_value_rule(), "{c:?} should not be value_rule");
        assert!(!c.is_predicate(), "{c:?} should not be predicate");
        assert!(!c.is_deferred(), "{c:?} should not be deferred");
    }
}

#[test]
fn all_value_rules_are_not_predicates() {
    let value_rules = vec![
        Rule::Pattern {
            pattern: "x".into(),
            message: None,
        },
        Rule::MinLength {
            min: 1,
            message: None,
        },
        Rule::MaxLength {
            max: 1,
            message: None,
        },
        Rule::Min {
            min: serde_json::Number::from(1),
            message: None,
        },
        Rule::Max {
            max: serde_json::Number::from(1),
            message: None,
        },
        Rule::OneOf {
            values: vec![],
            message: None,
        },
        Rule::MinItems {
            min: 1,
            message: None,
        },
        Rule::MaxItems {
            max: 1,
            message: None,
        },
        Rule::Email { message: None },
        Rule::Url { message: None },
    ];
    for r in &value_rules {
        assert!(r.is_value_rule(), "{r:?} should be value_rule");
        assert!(!r.is_predicate(), "{r:?} should not be predicate");
    }
}

#[test]
fn all_predicates_are_not_value_rules() {
    let predicates = vec![
        Rule::Eq {
            field: "x".into(),
            value: json!(1),
        },
        Rule::Ne {
            field: "x".into(),
            value: json!(1),
        },
        Rule::Gt {
            field: "x".into(),
            value: serde_json::Number::from(1),
        },
        Rule::Gte {
            field: "x".into(),
            value: serde_json::Number::from(1),
        },
        Rule::Lt {
            field: "x".into(),
            value: serde_json::Number::from(1),
        },
        Rule::Lte {
            field: "x".into(),
            value: serde_json::Number::from(1),
        },
        Rule::IsTrue { field: "x".into() },
        Rule::IsFalse { field: "x".into() },
        Rule::Set { field: "x".into() },
        Rule::Empty { field: "x".into() },
        Rule::Contains {
            field: "x".into(),
            value: json!(1),
        },
        Rule::Matches {
            field: "x".into(),
            pattern: "x".into(),
        },
        Rule::In {
            field: "x".into(),
            values: vec![],
        },
    ];
    for r in &predicates {
        assert!(r.is_predicate(), "{r:?} should be predicate");
        assert!(!r.is_value_rule(), "{r:?} should not be value_rule");
        assert!(!r.is_deferred(), "{r:?} should not be deferred");
    }
}

// ── Shorthand constructors ───────────────────────────────────────────

#[test]
fn shorthand_min_length() {
    let rule = Rule::min_length(5);
    assert!(matches!(
        rule,
        Rule::MinLength {
            min: 5,
            message: None
        }
    ));
}

#[test]
fn shorthand_pattern() {
    let rule = Rule::pattern(r"^\d+$");
    if let Rule::Pattern { pattern, message } = &rule {
        assert_eq!(pattern, r"^\d+$");
        assert!(message.is_none());
    } else {
        panic!("expected Pattern");
    }
}

#[test]
fn shorthand_with_message() {
    let rule = Rule::min_length(3).with_message("Too short");
    assert!(matches!(
        rule,
        Rule::MinLength {
            min: 3,
            message: Some(ref m),
        } if m == "Too short"
    ));
}

#[test]
fn shorthand_one_of() {
    let rule = Rule::one_of(["a", "b", "c"]);
    if let Rule::OneOf { values, message } = &rule {
        assert_eq!(values.len(), 3);
        assert!(message.is_none());
    } else {
        panic!("expected OneOf");
    }
}

#[test]
fn shorthand_min_max_value() {
    let min = Rule::min_value(0);
    let max = Rule::max_value(100);
    assert!(matches!(min, Rule::Min { .. }));
    assert!(matches!(max, Rule::Max { .. }));
}

#[test]
fn shorthand_all() {
    let rule = Rule::all([Rule::min_length(3), Rule::max_length(10)]);
    assert!(matches!(rule, Rule::All { rules } if rules.len() == 2));
}

#[test]
fn shorthand_any() {
    let rule = Rule::any([Rule::pattern("^a"), Rule::pattern("^b")]);
    assert!(matches!(rule, Rule::Any { rules } if rules.len() == 2));
}

#[test]
fn shorthand_not() {
    let rule = Rule::not(Rule::min_length(5));
    assert!(matches!(rule, Rule::Not { .. }));
}

#[test]
fn rule_implements_validate_trait() {
    use crate::foundation::Validate;

    let rule = Rule::min_length(3);
    assert!(rule.validate(&json!("alice")).is_ok());
    assert!(rule.validate(&json!("ab")).is_err());
}

#[test]
fn rule_composes_with_combinators() {
    use crate::foundation::{Validate, ValidateExt};

    let combined = Rule::min_length(3).and(Rule::max_length(10));
    assert!(combined.validate(&json!("hello")).is_ok());
    assert!(combined.validate(&json!("ab")).is_err());
    assert!(
        combined
            .validate(&json!("a very long string indeed"))
            .is_err()
    );
}

// ── Email / URL rules ───────────────────────────────────────────────

#[test]
fn email_rule_validates() {
    let rule = Rule::email();
    assert!(rule.validate_value(&json!("user@example.com")).is_ok());
    assert!(rule.validate_value(&json!("not-an-email")).is_err());
    // Non-string passes silently (consistent with other value rules)
    assert!(rule.validate_value(&json!(42)).is_ok());
}

#[test]
fn url_rule_validates() {
    let rule = Rule::url();
    assert!(rule.validate_value(&json!("https://example.com")).is_ok());
    assert!(rule.validate_value(&json!("not-a-url")).is_err());
    assert!(rule.validate_value(&json!(42)).is_ok());
}

#[test]
fn email_rule_with_custom_message() {
    let rule = Rule::email().with_message("Please enter a valid email");
    let err = rule.validate_value(&json!("bad")).unwrap_err();
    assert_eq!(err.message.as_ref(), "Please enter a valid email");
}
