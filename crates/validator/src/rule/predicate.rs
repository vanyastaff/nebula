//! Context predicates — boolean tests against sibling fields.
//!
//! Each variant holds a `FieldPath` to the sibling. `evaluate` takes a
//! `PredicateContext` and returns `bool`. Missing-field semantics are
//! documented per-variant.

use serde::{Deserialize, Serialize};

use super::{context::PredicateContext, helpers::compile_regex};
use crate::foundation::FieldPath;

/// Context predicate. Always evaluates to `bool` — never errors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Predicate {
    /// `field == value`
    Eq(FieldPath, serde_json::Value),
    /// `field != value`
    Ne(FieldPath, serde_json::Value),
    /// `field > value` (numeric).
    Gt(FieldPath, serde_json::Number),
    /// `field >= value` (numeric).
    Gte(FieldPath, serde_json::Number),
    /// `field < value` (numeric).
    Lt(FieldPath, serde_json::Number),
    /// `field <= value` (numeric).
    Lte(FieldPath, serde_json::Number),
    /// `field == true`
    IsTrue(FieldPath),
    /// `field == false`
    IsFalse(FieldPath),
    /// Field has a non-null, non-empty value.
    Set(FieldPath),
    /// Field is null, absent, or empty string/array.
    Empty(FieldPath),
    /// String or array field contains the given value.
    Contains(FieldPath, serde_json::Value),
    /// String field matches the regular expression.
    Matches(FieldPath, String),
    /// Field value is a member of the given set.
    In(FieldPath, Vec<serde_json::Value>),
}

impl Predicate {
    /// Returns the `FieldPath` this predicate references.
    pub fn field(&self) -> &FieldPath {
        match self {
            Self::Eq(f, _)
            | Self::Ne(f, _)
            | Self::Gt(f, _)
            | Self::Gte(f, _)
            | Self::Lt(f, _)
            | Self::Lte(f, _)
            | Self::IsTrue(f)
            | Self::IsFalse(f)
            | Self::Set(f)
            | Self::Empty(f)
            | Self::Contains(f, _)
            | Self::Matches(f, _)
            | Self::In(f, _) => f,
        }
    }

    /// Evaluates the predicate against context. Missing field → per-variant
    /// defaults: `Eq/Gt/Gte/Lt/Lte/IsTrue/IsFalse/Set/Contains/Matches/In` → false;
    /// `Ne` → true; `Empty` → true.
    #[must_use]
    pub fn evaluate(&self, ctx: &PredicateContext) -> bool {
        use super::helpers::cmp_number_predicate;

        match self {
            Self::Eq(f, v) => ctx.get(f).is_some_and(|x| x == v),
            Self::Ne(f, v) => ctx.get(f).is_none_or(|x| x != v),
            Self::Gt(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_gt()),
            Self::Gte(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_ge()),
            Self::Lt(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_lt()),
            Self::Lte(f, v) => cmp_number_predicate(ctx.get(f), v, |o| o.is_le()),
            Self::IsTrue(f) => ctx.get(f).and_then(serde_json::Value::as_bool) == Some(true),
            Self::IsFalse(f) => ctx.get(f).and_then(serde_json::Value::as_bool) == Some(false),
            Self::Set(f) => ctx.get(f).is_some_and(|v| {
                !v.is_null()
                    && match v {
                        serde_json::Value::String(s) => !s.is_empty(),
                        serde_json::Value::Array(a) => !a.is_empty(),
                        _ => true,
                    }
            }),
            Self::Empty(f) => ctx.get(f).is_none_or(|v| {
                v.is_null()
                    || match v {
                        serde_json::Value::String(s) => s.is_empty(),
                        serde_json::Value::Array(a) => a.is_empty(),
                        _ => false,
                    }
            }),
            Self::Contains(f, v) => ctx.get(f).is_some_and(|x| match x {
                serde_json::Value::String(s) => v.as_str().is_some_and(|needle| s.contains(needle)),
                serde_json::Value::Array(items) => items.contains(v),
                _ => false,
            }),
            Self::Matches(f, pat) => {
                debug_assert!(
                    regex::Regex::new(pat).is_ok(),
                    "Predicate::Matches: invalid regex {pat:?}"
                );
                ctx.get(f)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| compile_regex(pat).is_ok_and(|re| re.is_match(s)))
            },
            Self::In(f, allowed) => ctx.get(f).is_some_and(|x| allowed.contains(x)),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ctx_from(obj: serde_json::Value) -> PredicateContext {
        PredicateContext::from_json(&obj)
    }

    #[test]
    fn eq_matches_existing_field() {
        let p = Predicate::Eq(FieldPath::parse("status").unwrap(), json!("active"));
        assert!(p.evaluate(&ctx_from(json!({"status": "active"}))));
    }

    #[test]
    fn eq_on_missing_field_is_false() {
        let p = Predicate::Eq(FieldPath::parse("status").unwrap(), json!("active"));
        assert!(!p.evaluate(&ctx_from(json!({}))));
    }

    #[test]
    fn ne_on_missing_field_is_true() {
        let p = Predicate::Ne(FieldPath::parse("status").unwrap(), json!("active"));
        assert!(p.evaluate(&ctx_from(json!({}))));
    }

    #[test]
    fn wire_form_tuple_is_compact() {
        let p = Predicate::Eq(FieldPath::parse("status").unwrap(), json!("active"));
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j, json!({"eq": ["/status", "active"]}));
    }

    #[test]
    fn wire_form_roundtrip() {
        let p = Predicate::In(
            FieldPath::parse("method").unwrap(),
            vec![json!("POST"), json!("PUT")],
        );
        let back: Predicate = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }
}
