//! [`Rule::evaluate`] — boolean predicate evaluation against field context.

use super::{
    Rule,
    helpers::{cmp_number_predicate, compile_regex},
};

impl Rule {
    /// Evaluates this rule as a boolean predicate against field values.
    ///
    /// Only meaningful for context-predicate rules and logical combinators.
    /// Value-validation rules return `true` (vacuously — use
    /// [`validate_value`](Self::validate_value) instead).
    ///
    /// # Missing Fields
    ///
    /// Behavior when the target field is absent:
    /// - `Eq` → `false` (can't equal anything)
    /// - `Ne` → `true` (absent ≠ any value)
    /// - `Gt`, `Gte`, `Lt`, `Lte` → `false` (no number to compare)
    /// - `IsTrue`, `IsFalse` → `false` (no boolean)
    /// - `Set` → `false`, `Empty` → `true`
    /// - `Contains`, `Matches`, `In` → `false`
    #[must_use]
    pub fn evaluate(&self, values: &std::collections::HashMap<String, serde_json::Value>) -> bool {
        match self {
            // ── Context predicates ──────────────────────────────────
            Self::Eq { field, value } => values.get(field).is_some_and(|v| v == value),
            Self::Ne { field, value } => values.get(field).is_none_or(|v| v != value),
            Self::Gt { field, value } => {
                cmp_number_predicate(values.get(field), value, |o| o.is_gt())
            },
            Self::Gte { field, value } => {
                cmp_number_predicate(values.get(field), value, |o| o.is_ge())
            },
            Self::Lt { field, value } => {
                cmp_number_predicate(values.get(field), value, |o| o.is_lt())
            },
            Self::Lte { field, value } => {
                cmp_number_predicate(values.get(field), value, |o| o.is_le())
            },
            Self::IsTrue { field } => {
                values.get(field).and_then(serde_json::Value::as_bool) == Some(true)
            },
            Self::IsFalse { field } => {
                values.get(field).and_then(serde_json::Value::as_bool) == Some(false)
            },
            Self::Set { field } => values.get(field).is_some_and(|v| {
                !v.is_null()
                    && match v {
                        serde_json::Value::String(s) => !s.is_empty(),
                        serde_json::Value::Array(a) => !a.is_empty(),
                        _ => true,
                    }
            }),
            Self::Empty { field } => values.get(field).is_none_or(|v| {
                v.is_null()
                    || match v {
                        serde_json::Value::String(s) => s.is_empty(),
                        serde_json::Value::Array(a) => a.is_empty(),
                        _ => false,
                    }
            }),
            Self::Contains { field, value } => values.get(field).is_some_and(|v| match v {
                serde_json::Value::String(s) => {
                    value.as_str().is_some_and(|needle| s.contains(needle))
                },
                serde_json::Value::Array(items) => items.contains(value),
                _ => false,
            }),
            Self::Matches { field, pattern } => {
                debug_assert!(
                    regex::Regex::new(pattern).is_ok(),
                    "Rule::Matches: invalid regex pattern '{pattern}' — evaluate() will always return false"
                );
                values
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|string| {
                        compile_regex(pattern).is_ok_and(|re| re.is_match(string))
                    })
            },
            Self::In {
                field,
                values: candidates,
            } => values
                .get(field)
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
