//! Unified declarative rule system.
//!
//! [`Rule`] is the single source of truth for both value validation and
//! context-predicate evaluation. Adding a new rule means adding one
//! enum variant and one match arm — everything else works automatically.
//!
//! # Rule categories
//!
//! | Category | Examples | Method |
//! |---|---|---|
//! | Value validation | `MinLength`, `Max`, `Pattern` | [`Rule::validate_value`] |
//! | Context predicate | `Eq`, `Set`, `IsTrue` | [`Rule::evaluate`] |
//! | Deferred | `Custom`, `UniqueBy` | skipped at schema time |
//! | Logical combinator | `All`, `Any`, `Not` | both methods |
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::Rule;
//! use serde_json::json;
//!
//! // Value validation
//! let rule = Rule::MinLength { min: 3, message: None };
//! assert!(rule.validate_value(&json!("alice")).is_ok());
//! assert!(rule.validate_value(&json!("ab")).is_err());
//!
//! // Context predicate
//! use nebula_validator::FieldValueProvider;
//! use std::collections::HashMap;
//!
//! let rule = Rule::Eq {
//!     field: "status".into(),
//!     value: json!("active"),
//! };
//! let mut values = HashMap::new();
//! values.insert("status".into(), json!("active"));
//! assert!(rule.evaluate(&values));
//! ```

use serde::{Deserialize, Serialize};

use crate::foundation::{Validate, ValidationError};
use crate::validators::{matches_regex, max, max_length, max_size, min, min_length, min_size};

use crate::context::FieldValueProvider;

/// Unified declarative rule.
///
/// Covers value validation, context predicates, deferred runtime checks,
/// and logical combinators. One enum, one source of truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Rule {
    // ── Value validation rules ──────────────────────────────────────────
    /// String must match the regular expression.
    Pattern {
        /// Regular expression pattern.
        pattern: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// String must be at least `min` characters.
    MinLength {
        /// Minimum character count (inclusive).
        min: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// String must be at most `max` characters.
    MaxLength {
        /// Maximum character count (inclusive).
        max: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Number must be ≥ `min`.
    Min {
        /// Lower bound (inclusive).
        min: serde_json::Number,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Number must be ≤ `max`.
    Max {
        /// Upper bound (inclusive).
        max: serde_json::Number,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Value must be one of the given options.
    OneOf {
        /// Allowed values.
        values: Vec<serde_json::Value>,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Collection must contain at least `min` items.
    MinItems {
        /// Minimum item count (inclusive).
        min: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Collection must contain at most `max` items.
    MaxItems {
        /// Maximum item count (inclusive).
        max: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    // ── Deferred rules (runtime only) ───────────────────────────────────
    /// Each list item must have a unique value for the given sub-field key.
    ///
    /// **Deferred**: not evaluated at schema-validation time.
    UniqueBy {
        /// Sub-field key path within each item (e.g. `"name"`).
        key: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Custom expression-based validation.
    ///
    /// **Deferred**: not evaluated at schema-validation time.
    Custom {
        /// Expression string forwarded to the runtime evaluator.
        expression: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    // ── Context predicates (field-level conditions) ─────────────────────
    /// `field == value`
    Eq {
        /// Field id to read.
        field: String,
        /// Expected value.
        value: serde_json::Value,
    },

    /// `field != value`
    Ne {
        /// Field id to read.
        field: String,
        /// Value to differ from.
        value: serde_json::Value,
    },

    /// `field > value` (numeric comparison)
    Gt {
        /// Field id to read.
        field: String,
        /// Lower exclusive bound.
        value: serde_json::Number,
    },

    /// `field >= value` (numeric comparison)
    Gte {
        /// Field id to read.
        field: String,
        /// Lower inclusive bound.
        value: serde_json::Number,
    },

    /// `field < value` (numeric comparison)
    Lt {
        /// Field id to read.
        field: String,
        /// Upper exclusive bound.
        value: serde_json::Number,
    },

    /// `field <= value` (numeric comparison)
    Lte {
        /// Field id to read.
        field: String,
        /// Upper inclusive bound.
        value: serde_json::Number,
    },

    /// `field == true`
    IsTrue {
        /// Field id to read.
        field: String,
    },

    /// `field == false`
    IsFalse {
        /// Field id to read.
        field: String,
    },

    /// Field has a non-null, non-empty value.
    Set {
        /// Field id to read.
        field: String,
    },

    /// Field is null, absent, or empty string/array.
    Empty {
        /// Field id to read.
        field: String,
    },

    /// String or array field contains the given value.
    Contains {
        /// Field id to read.
        field: String,
        /// Value to search for.
        value: serde_json::Value,
    },

    /// String field matches the regular expression.
    Matches {
        /// Field id to read.
        field: String,
        /// Regular expression pattern.
        pattern: String,
    },

    /// Field value is a member of the given set.
    In {
        /// Field id to read.
        field: String,
        /// Allowed values.
        values: Vec<serde_json::Value>,
    },

    // ── Logical combinators ─────────────────────────────────────────────
    /// All inner rules must pass.
    All {
        /// Inner rules.
        rules: Vec<Rule>,
    },

    /// At least one inner rule must pass.
    Any {
        /// Inner rules.
        rules: Vec<Rule>,
    },

    /// Negates the inner rule.
    Not {
        /// Inner rule to negate.
        inner: Box<Rule>,
    },
}

impl Rule {
    /// Returns `true` if this rule validates a single value
    /// (as opposed to evaluating context predicates).
    #[must_use]
    pub fn is_value_rule(&self) -> bool {
        matches!(
            self,
            Self::Pattern { .. }
                | Self::MinLength { .. }
                | Self::MaxLength { .. }
                | Self::Min { .. }
                | Self::Max { .. }
                | Self::OneOf { .. }
                | Self::MinItems { .. }
                | Self::MaxItems { .. }
                | Self::UniqueBy { .. }
                | Self::Custom { .. }
        )
    }

    /// Returns `true` if this rule evaluates context predicates
    /// (checks a sibling field value).
    #[must_use]
    pub fn is_predicate(&self) -> bool {
        matches!(
            self,
            Self::Eq { .. }
                | Self::Ne { .. }
                | Self::Gt { .. }
                | Self::Gte { .. }
                | Self::Lt { .. }
                | Self::Lte { .. }
                | Self::IsTrue { .. }
                | Self::IsFalse { .. }
                | Self::Set { .. }
                | Self::Empty { .. }
                | Self::Contains { .. }
                | Self::Matches { .. }
                | Self::In { .. }
        )
    }

    /// Returns `true` if this rule requires runtime expression context.
    ///
    /// Deferred rules are skipped during static schema validation.
    #[must_use]
    pub fn is_deferred(&self) -> bool {
        matches!(self, Self::UniqueBy { .. } | Self::Custom { .. })
    }

    /// Validates a JSON value against this rule.
    ///
    /// Only meaningful for value-validation rules. Predicate rules
    /// return `Ok(())` (use [`evaluate`](Self::evaluate) instead).
    /// Deferred rules return `Ok(())` (skipped at static time).
    pub fn validate_value(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        match self {
            // ── Value rules ─────────────────────────────────────────
            Self::MinLength {
                min: min_val,
                message,
            } => {
                if let Some(s) = value.as_str() {
                    min_length(*min_val)
                        .validate(s)
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            }
            Self::MaxLength {
                max: max_val,
                message,
            } => {
                if let Some(s) = value.as_str() {
                    max_length(*max_val)
                        .validate(s)
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            }
            Self::Pattern { pattern, message } => {
                if let Some(s) = value.as_str() {
                    let validator = matches_regex(pattern).map_err(|e| {
                        ValidationError::new("invalid_pattern", format!("invalid regex: {e}"))
                    })?;
                    validator
                        .validate(s)
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            }
            Self::Min {
                min: min_val,
                message,
            } => {
                if let (Some(current), Some(bound)) = (value.as_f64(), min_val.as_f64()) {
                    min(bound)
                        .validate(&current)
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            }
            Self::Max {
                max: max_val,
                message,
            } => {
                if let (Some(current), Some(bound)) = (value.as_f64(), max_val.as_f64()) {
                    max(bound)
                        .validate(&current)
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            }
            Self::OneOf { values, message } => {
                if !values.contains(value) {
                    let msg = message
                        .clone()
                        .unwrap_or_else(|| "must be one of the allowed values".to_owned());
                    return Err(ValidationError::new("one_of", msg));
                }
                Ok(())
            }
            Self::MinItems {
                min: min_val,
                message,
            } => {
                if let Some(items) = value.as_array() {
                    min_size::<serde_json::Value>(*min_val)
                        .validate(items.as_slice())
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            }
            Self::MaxItems {
                max: max_val,
                message,
            } => {
                if let Some(items) = value.as_array() {
                    max_size::<serde_json::Value>(*max_val)
                        .validate(items.as_slice())
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            }

            // ── Deferred — skip at static time ──────────────────────
            Self::UniqueBy { .. } | Self::Custom { .. } => Ok(()),

            // ── Context predicates — not value checks ───────────────
            Self::Eq { .. }
            | Self::Ne { .. }
            | Self::Gt { .. }
            | Self::Gte { .. }
            | Self::Lt { .. }
            | Self::Lte { .. }
            | Self::IsTrue { .. }
            | Self::IsFalse { .. }
            | Self::Set { .. }
            | Self::Empty { .. }
            | Self::Contains { .. }
            | Self::Matches { .. }
            | Self::In { .. } => Ok(()),

            // ── Logical combinators ─────────────────────────────────
            Self::All { rules } => {
                for rule in rules {
                    rule.validate_value(value)?;
                }
                Ok(())
            }
            Self::Any { rules } => {
                if rules.is_empty() {
                    return Ok(());
                }
                let mut last_err = None;
                for rule in rules {
                    match rule.validate_value(value) {
                        Ok(()) => return Ok(()),
                        Err(e) => last_err = Some(e),
                    }
                }
                Err(last_err.unwrap_or_else(|| {
                    ValidationError::new("any_failed", "none of the rules passed")
                }))
            }
            Self::Not { inner } => match inner.validate_value(value) {
                Ok(()) => Err(ValidationError::new("not_failed", "negated rule passed")),
                Err(_) => Ok(()),
            },
        }
    }

    /// Evaluates this rule as a boolean predicate against field values.
    ///
    /// Only meaningful for context-predicate rules and logical combinators.
    /// Value-validation rules return `true` (vacuously — use
    /// [`validate_value`](Self::validate_value) instead).
    #[must_use]
    pub fn evaluate(&self, values: &impl FieldValueProvider) -> bool {
        match self {
            // ── Context predicates ──────────────────────────────────
            Self::Eq { field, value } => values.get_field(field).is_some_and(|v| v == value),
            Self::Ne { field, value } => values.get_field(field).is_none_or(|v| v != value),
            Self::Gt { field, value } => cmp_number(values.get_field(field), value, |a, b| a > b),
            Self::Gte { field, value } => cmp_number(values.get_field(field), value, |a, b| a >= b),
            Self::Lt { field, value } => cmp_number(values.get_field(field), value, |a, b| a < b),
            Self::Lte { field, value } => cmp_number(values.get_field(field), value, |a, b| a <= b),
            Self::IsTrue { field } => {
                values.get_field(field).and_then(serde_json::Value::as_bool) == Some(true)
            }
            Self::IsFalse { field } => {
                values.get_field(field).and_then(serde_json::Value::as_bool) == Some(false)
            }
            Self::Set { field } => values.get_field(field).is_some_and(|v| {
                !v.is_null()
                    && match v {
                        serde_json::Value::String(s) => !s.is_empty(),
                        serde_json::Value::Array(a) => !a.is_empty(),
                        _ => true,
                    }
            }),
            Self::Empty { field } => values.get_field(field).is_none_or(|v| {
                v.is_null()
                    || match v {
                        serde_json::Value::String(s) => s.is_empty(),
                        serde_json::Value::Array(a) => a.is_empty(),
                        _ => false,
                    }
            }),
            Self::Contains { field, value } => values.get_field(field).is_some_and(|v| match v {
                serde_json::Value::String(s) => {
                    value.as_str().is_some_and(|needle| s.contains(needle))
                }
                serde_json::Value::Array(items) => items.contains(value),
                _ => false,
            }),
            Self::Matches { field, pattern } => values
                .get_field(field)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|string| {
                    matches_regex(pattern).is_ok_and(|validator| validator.validate(string).is_ok())
                }),
            Self::In {
                field,
                values: candidates,
            } => values
                .get_field(field)
                .is_some_and(|current| candidates.contains(current)),

            // ── Logical combinators ─────────────────────────────────
            Self::All { rules } => rules.iter().all(|r| r.evaluate(values)),
            Self::Any { rules } => rules.iter().any(|r| r.evaluate(values)),
            Self::Not { inner } => !inner.evaluate(values),

            // ── Value/deferred rules — vacuously true ───────────────
            _ => true,
        }
    }
}

/// Replaces the error message if an override is provided.
fn override_message(mut error: ValidationError, message: &Option<String>) -> ValidationError {
    if let Some(msg) = message {
        error.message = std::borrow::Cow::Owned(msg.clone());
    }
    error
}

/// Numeric comparison helper.
fn cmp_number(
    value: Option<&serde_json::Value>,
    rhs: &serde_json::Number,
    op: impl Fn(f64, f64) -> bool,
) -> bool {
    let Some(lhs) = value.and_then(serde_json::Value::as_f64) else {
        return false;
    };
    let Some(rhs) = rhs.as_f64() else {
        return false;
    };
    op(lhs, rhs)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

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
    fn is_true_predicate() {
        let rule = Rule::IsTrue {
            field: "enabled".into(),
        };
        assert!(rule.evaluate(&values(&[("enabled", json!(true))])));
        assert!(!rule.evaluate(&values(&[("enabled", json!(false))])));
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
}
