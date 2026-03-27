//! Declarative conditions for parameter visibility and validation.
//!
//! [`Condition`] expresses when a parameter should be shown, required, or
//! otherwise active.  Conditions form a tree: leaf variants test a single
//! field, while [`All`](Condition::All), [`Any`](Condition::Any), and
//! [`Not`](Condition::Not) compose them into arbitrary boolean logic.
//!
//! # Examples
//!
//! ```
//! use nebula_parameter::conditions::Condition;
//! use nebula_parameter::path::ParameterPath;
//!
//! let cond = Condition::all(vec![
//!     Condition::eq("auth_mode", "oauth2"),
//!     Condition::set("client_id"),
//! ]);
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::path::ParameterPath;

/// A declarative predicate over runtime parameter values.
///
/// Leaf variants reference a single field via [`ParameterPath`]; logical
/// combinators ([`All`](Self::All), [`Any`](Self::Any), [`Not`](Self::Not))
/// compose sub-conditions into arbitrary boolean expressions.
///
/// Serialized with `"op"` as the tag field and `snake_case` variant names,
/// so `Condition::Eq { .. }` becomes `{ "op": "eq", "field": "...", "value": ... }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    /// True when the field value equals `value`.
    Eq {
        /// The parameter to test.
        field: ParameterPath,
        /// The expected value.
        value: Value,
    },

    /// True when the field value does **not** equal `value`.
    ///
    /// A missing field is considered not-equal.
    Ne {
        /// The parameter to test.
        field: ParameterPath,
        /// The value to reject.
        value: Value,
    },

    /// True when the field value is one of the listed `values`.
    OneOf {
        /// The parameter to test.
        field: ParameterPath,
        /// Accepted values.
        values: Vec<Value>,
    },

    /// True when the field exists and is not `null`.
    Set {
        /// The parameter to test.
        field: ParameterPath,
    },

    /// True when the field is absent or `null`.
    NotSet {
        /// The parameter to test.
        field: ParameterPath,
    },

    /// True when the field value is the boolean `true`.
    IsTrue {
        /// The parameter to test.
        field: ParameterPath,
    },

    /// True when the field value is numerically greater than `value`.
    ///
    /// Both the field and `value` must be representable as `f64`.
    Gt {
        /// The parameter to test.
        field: ParameterPath,
        /// The threshold (exclusive lower bound).
        value: Value,
    },

    /// True when the field value is numerically less than `value`.
    ///
    /// Both the field and `value` must be representable as `f64`.
    Lt {
        /// The parameter to test.
        field: ParameterPath,
        /// The threshold (exclusive upper bound).
        value: Value,
    },

    /// True when **all** sub-conditions are true (logical AND).
    All {
        /// The sub-conditions to evaluate.
        conditions: Vec<Condition>,
    },

    /// True when **any** sub-condition is true (logical OR).
    Any {
        /// The sub-conditions to evaluate.
        conditions: Vec<Condition>,
    },

    /// True when the inner condition is false (logical NOT).
    Not {
        /// The condition to negate.
        condition: Box<Condition>,
    },
}

impl Condition {
    // ── Shorthand constructors ──────────────────────────────────────────

    /// Create an [`Eq`](Self::Eq) condition.
    #[must_use]
    pub fn eq(field: impl Into<ParameterPath>, value: impl Into<Value>) -> Self {
        Self::Eq {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Create a [`Ne`](Self::Ne) condition.
    #[must_use]
    pub fn ne(field: impl Into<ParameterPath>, value: impl Into<Value>) -> Self {
        Self::Ne {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Create a [`OneOf`](Self::OneOf) condition.
    #[must_use]
    pub fn one_of<V: Into<Value>>(
        field: impl Into<ParameterPath>,
        values: impl IntoIterator<Item = V>,
    ) -> Self {
        Self::OneOf {
            field: field.into(),
            values: values.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a [`Set`](Self::Set) condition.
    #[must_use]
    pub fn set(field: impl Into<ParameterPath>) -> Self {
        Self::Set {
            field: field.into(),
        }
    }

    /// Create a [`NotSet`](Self::NotSet) condition.
    #[must_use]
    pub fn not_set(field: impl Into<ParameterPath>) -> Self {
        Self::NotSet {
            field: field.into(),
        }
    }

    /// Create an [`All`](Self::All) condition (logical AND).
    #[must_use]
    pub fn all(conditions: Vec<Self>) -> Self {
        Self::All { conditions }
    }

    /// Create an [`Any`](Self::Any) condition (logical OR).
    #[must_use]
    pub fn any(conditions: Vec<Self>) -> Self {
        Self::Any { conditions }
    }

    /// Create a [`Not`](Self::Not) condition (logical negation).
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn not(condition: Self) -> Self {
        Self::Not {
            condition: Box::new(condition),
        }
    }

    /// Create a [`Gt`](Self::Gt) condition (numeric greater-than).
    #[must_use]
    pub fn gt(field: impl Into<ParameterPath>, value: impl Into<Value>) -> Self {
        Self::Gt {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Create a [`Lt`](Self::Lt) condition (numeric less-than).
    #[must_use]
    pub fn lt(field: impl Into<ParameterPath>, value: impl Into<Value>) -> Self {
        Self::Lt {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Create an [`IsTrue`](Self::IsTrue) condition.
    #[must_use]
    pub fn is_true(field: impl Into<ParameterPath>) -> Self {
        Self::IsTrue {
            field: field.into(),
        }
    }

    // ── Evaluation ──────────────────────────────────────────────────────

    /// Evaluate this condition against a set of runtime values.
    ///
    /// Keys in `values` are matched against the field's
    /// [`ParameterPath::as_str`] representation.
    #[must_use]
    pub fn evaluate(&self, values: &HashMap<String, Value>) -> bool {
        match self {
            Self::Eq { field, value } => values.get(field.as_str()) == Some(value),

            Self::Ne { field, value } => values.get(field.as_str()) != Some(value),

            Self::OneOf { field, values: vs } => {
                values.get(field.as_str()).is_some_and(|v| vs.contains(v))
            }

            Self::Set { field } => values.get(field.as_str()).is_some_and(|v| !v.is_null()),

            Self::NotSet { field } => values.get(field.as_str()).is_none_or(Value::is_null),

            Self::IsTrue { field } => {
                values.get(field.as_str()).and_then(Value::as_bool) == Some(true)
            }

            Self::Gt { field, value } => match (
                values.get(field.as_str()).and_then(Value::as_f64),
                value.as_f64(),
            ) {
                (Some(lhs), Some(rhs)) => lhs > rhs,
                _ => false,
            },

            Self::Lt { field, value } => match (
                values.get(field.as_str()).and_then(Value::as_f64),
                value.as_f64(),
            ) {
                (Some(lhs), Some(rhs)) => lhs < rhs,
                _ => false,
            },

            Self::All { conditions } => conditions.iter().all(|c| c.evaluate(values)),

            Self::Any { conditions } => conditions.iter().any(|c| c.evaluate(values)),

            Self::Not { condition } => !condition.evaluate(values),
        }
    }

    // ── Introspection ───────────────────────────────────────────────────

    /// Collect all field references reachable from this condition.
    ///
    /// Walks the condition tree and appends each leaf field's string
    /// representation to `refs`.  Useful for dependency analysis and
    /// topological ordering of parameters.
    pub fn field_references<'a>(&'a self, refs: &mut Vec<&'a str>) {
        match self {
            Self::Eq { field, .. }
            | Self::Ne { field, .. }
            | Self::OneOf { field, .. }
            | Self::Set { field }
            | Self::NotSet { field }
            | Self::IsTrue { field }
            | Self::Gt { field, .. }
            | Self::Lt { field, .. } => {
                refs.push(field.as_str());
            }
            Self::All { conditions } | Self::Any { conditions } => {
                for c in conditions {
                    c.field_references(refs);
                }
            }
            Self::Not { condition } => {
                condition.field_references(refs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn eq_matches_exact_value() {
        let cond = Condition::eq("mode", "fast");
        let vals = values(&[("mode", Value::String("fast".into()))]);
        assert!(cond.evaluate(&vals));
    }

    #[test]
    fn eq_rejects_different_value() {
        let cond = Condition::eq("mode", "fast");
        let vals = values(&[("mode", Value::String("slow".into()))]);
        assert!(!cond.evaluate(&vals));
    }

    #[test]
    fn eq_rejects_absent_field() {
        let cond = Condition::eq("mode", "fast");
        assert!(!cond.evaluate(&HashMap::new()));
    }

    #[test]
    fn ne_rejects_matching_value() {
        let cond = Condition::ne("mode", "fast");
        let vals = values(&[("mode", Value::String("fast".into()))]);
        assert!(!cond.evaluate(&vals));
    }

    #[test]
    fn ne_accepts_absent_field() {
        let cond = Condition::ne("mode", "fast");
        assert!(cond.evaluate(&HashMap::new()));
    }

    #[test]
    fn one_of_matches_contained_value() {
        let cond = Condition::one_of(
            "color",
            vec![Value::String("red".into()), Value::String("blue".into())],
        );
        let vals = values(&[("color", Value::String("blue".into()))]);
        assert!(cond.evaluate(&vals));
    }

    #[test]
    fn one_of_rejects_missing_value() {
        let cond = Condition::one_of(
            "color",
            vec![Value::String("red".into()), Value::String("blue".into())],
        );
        let vals = values(&[("color", Value::String("green".into()))]);
        assert!(!cond.evaluate(&vals));
    }

    #[test]
    fn set_detects_present_non_null() {
        let cond = Condition::set("name");
        let vals = values(&[("name", Value::String("Alice".into()))]);
        assert!(cond.evaluate(&vals));
    }

    #[test]
    fn set_rejects_null() {
        let cond = Condition::set("name");
        let vals = values(&[("name", Value::Null)]);
        assert!(!cond.evaluate(&vals));
    }

    #[test]
    fn set_rejects_absent() {
        let cond = Condition::set("name");
        assert!(!cond.evaluate(&HashMap::new()));
    }

    #[test]
    fn not_set_accepts_absent() {
        let cond = Condition::not_set("name");
        assert!(cond.evaluate(&HashMap::new()));
    }

    #[test]
    fn not_set_accepts_null() {
        let cond = Condition::not_set("name");
        let vals = values(&[("name", Value::Null)]);
        assert!(cond.evaluate(&vals));
    }

    #[test]
    fn is_true_matches_boolean_true() {
        let cond = Condition::is_true("enabled");
        let vals = values(&[("enabled", Value::Bool(true))]);
        assert!(cond.evaluate(&vals));
    }

    #[test]
    fn is_true_rejects_false() {
        let cond = Condition::is_true("enabled");
        let vals = values(&[("enabled", Value::Bool(false))]);
        assert!(!cond.evaluate(&vals));
    }

    #[test]
    fn gt_compares_numerically() {
        let cond = Condition::gt("count", 5);
        let above = values(&[("count", serde_json::json!(10))]);
        let below = values(&[("count", serde_json::json!(3))]);
        assert!(cond.evaluate(&above));
        assert!(!cond.evaluate(&below));
    }

    #[test]
    fn lt_compares_numerically() {
        let cond = Condition::lt("count", 5);
        let below = values(&[("count", serde_json::json!(3))]);
        let above = values(&[("count", serde_json::json!(10))]);
        assert!(cond.evaluate(&below));
        assert!(!cond.evaluate(&above));
    }

    #[test]
    fn all_requires_every_sub_condition() {
        let cond = Condition::all(vec![Condition::set("a"), Condition::set("b")]);
        let both = values(&[("a", Value::Bool(true)), ("b", Value::Bool(true))]);
        let one = values(&[("a", Value::Bool(true))]);
        assert!(cond.evaluate(&both));
        assert!(!cond.evaluate(&one));
    }

    #[test]
    fn any_requires_at_least_one() {
        let cond = Condition::any(vec![Condition::set("a"), Condition::set("b")]);
        let one = values(&[("b", Value::Bool(true))]);
        assert!(cond.evaluate(&one));
        assert!(!cond.evaluate(&HashMap::new()));
    }

    #[test]
    fn not_negates_inner() {
        let cond = Condition::not(Condition::set("x"));
        assert!(cond.evaluate(&HashMap::new()));
        let vals = values(&[("x", Value::Bool(true))]);
        assert!(!cond.evaluate(&vals));
    }

    #[test]
    fn field_references_collects_all_leaves() {
        let cond = Condition::all(vec![
            Condition::eq("a", "v"),
            Condition::not(Condition::set("b")),
            Condition::any(vec![Condition::is_true("c")]),
        ]);
        let mut refs = Vec::new();
        cond.field_references(&mut refs);
        assert_eq!(refs, vec!["a", "b", "c"]);
    }

    #[test]
    fn one_of_accepts_string_slices() {
        let cond = Condition::one_of("color", ["red", "blue"]);
        let vals = values(&[("color", Value::String("blue".into()))]);
        assert!(cond.evaluate(&vals));
    }

    #[test]
    fn one_of_accepts_integers() {
        let cond = Condition::one_of("count", [1, 2, 3]);
        let vals = values(&[("count", serde_json::json!(2))]);
        assert!(cond.evaluate(&vals));
    }

    #[test]
    fn serde_round_trip() {
        let cond = Condition::eq("mode", "fast");
        let json = serde_json::to_string(&cond).expect("serialize");
        let back: Condition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cond, back);
    }

    #[test]
    fn serde_tag_format() {
        let cond = Condition::eq("mode", "fast");
        let v: Value = serde_json::to_value(&cond).expect("to_value");
        assert_eq!(v["op"], "eq");
        assert_eq!(v["field"], "mode");
    }
}
