//! Declarative conditions evaluated against live value maps.

use nebula_validator::foundation::Validate;
use nebula_validator::validators::matches_regex;

use crate::values::ParameterValues;

/// Deterministic condition evaluated against a live value map.
///
/// Used to drive field visibility, conditional-required logic, and
/// disabled state at both schema-definition time and runtime.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
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
    /// `field > value`
    Gt {
        /// Field id to read.
        field: String,
        /// Lower exclusive bound.
        value: serde_json::Number,
    },
    /// `field >= value`
    Gte {
        /// Field id to read.
        field: String,
        /// Lower inclusive bound.
        value: serde_json::Number,
    },
    /// `field < value`
    Lt {
        /// Field id to read.
        field: String,
        /// Upper exclusive bound.
        value: serde_json::Number,
    },
    /// `field <= value`
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
    /// All inner conditions must hold.
    All {
        /// Inner conditions.
        conditions: Vec<Condition>,
    },
    /// At least one inner condition must hold.
    Any {
        /// Inner conditions.
        conditions: Vec<Condition>,
    },
    /// Negates the inner condition.
    Not {
        /// Inner condition to negate.
        condition: Box<Condition>,
    },
}

/// Evaluate a declarative [`Condition`] against runtime values.
#[must_use]
pub fn evaluate_condition(condition: &Condition, values: &ParameterValues) -> bool {
    match condition {
        Condition::Eq { field, value } => values.get(field).is_some_and(|v| v == value),
        Condition::Ne { field, value } => values.get(field).is_none_or(|v| v != value),
        Condition::Gt { field, value } => cmp_number(values.get(field), value, |a, b| a > b),
        Condition::Gte { field, value } => cmp_number(values.get(field), value, |a, b| a >= b),
        Condition::Lt { field, value } => cmp_number(values.get(field), value, |a, b| a < b),
        Condition::Lte { field, value } => cmp_number(values.get(field), value, |a, b| a <= b),
        Condition::IsTrue { field } => {
            values.get(field).and_then(serde_json::Value::as_bool) == Some(true)
        }
        Condition::IsFalse { field } => {
            values.get(field).and_then(serde_json::Value::as_bool) == Some(false)
        }
        Condition::Set { field } => values.get(field).is_some_and(|v| {
            !v.is_null()
                && match v {
                    serde_json::Value::String(s) => !s.is_empty(),
                    serde_json::Value::Array(a) => !a.is_empty(),
                    _ => true,
                }
        }),
        Condition::Empty { field } => values.get(field).is_none_or(|v| {
            v.is_null()
                || match v {
                    serde_json::Value::String(s) => s.is_empty(),
                    serde_json::Value::Array(a) => a.is_empty(),
                    _ => false,
                }
        }),
        Condition::Contains { field, value } => values.get(field).is_some_and(|v| match v {
            serde_json::Value::String(s) => value.as_str().is_some_and(|needle| s.contains(needle)),
            serde_json::Value::Array(items) => items.contains(value),
            _ => false,
        }),
        Condition::Matches { field, pattern } => values
            .get(field)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|string| {
                matches_regex(pattern).is_ok_and(|validator| validator.validate(string).is_ok())
            }),
        Condition::In {
            field,
            values: candidates,
        } => values
            .get(field)
            .is_some_and(|current| candidates.contains(current)),
        Condition::All { conditions } => conditions
            .iter()
            .all(|nested| evaluate_condition(nested, values)),
        Condition::Any { conditions } => conditions
            .iter()
            .any(|nested| evaluate_condition(nested, values)),
        Condition::Not { condition } => !evaluate_condition(condition, values),
    }
}

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
