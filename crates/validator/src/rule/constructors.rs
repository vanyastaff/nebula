//! Ergonomic constructors for [`Rule`] and inner sub-enums.

use super::{DeferredRule, Logic, Predicate, Rule, ValueRule};
use crate::foundation::FieldPath;

// ── ValueRule ────────────────────────────────────────────────────────────
impl ValueRule {
    /// Creates a [`ValueRule::MinLength`].
    #[must_use]
    pub fn min_length(n: usize) -> Self {
        Self::MinLength(n)
    }
    /// Creates a [`ValueRule::MaxLength`].
    #[must_use]
    pub fn max_length(n: usize) -> Self {
        Self::MaxLength(n)
    }
    /// Creates a [`ValueRule::Pattern`].
    #[must_use]
    pub fn pattern(p: impl Into<String>) -> Self {
        Self::Pattern(p.into())
    }
}

// ── Predicate ────────────────────────────────────────────────────────────
impl Predicate {
    /// Creates a [`Predicate::Eq`]. Returns `None` if the path is invalid.
    #[must_use]
    pub fn eq(field: impl AsRef<str>, value: impl Into<serde_json::Value>) -> Option<Self> {
        Some(Self::Eq(FieldPath::parse(field)?, value.into()))
    }
}

// ── Rule: shorthand wrappers ─────────────────────────────────────────────
impl Rule {
    /// Wraps a [`ValueRule`] into [`Rule::Value`].
    #[must_use]
    pub fn value(v: ValueRule) -> Self {
        Self::Value(v)
    }

    /// Wraps a [`Predicate`] into [`Rule::Predicate`].
    #[must_use]
    pub fn predicate(p: Predicate) -> Self {
        Self::Predicate(p)
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::MinLength`].
    #[must_use]
    pub fn min_length(n: usize) -> Self {
        Self::Value(ValueRule::MinLength(n))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::MaxLength`].
    #[must_use]
    pub fn max_length(n: usize) -> Self {
        Self::Value(ValueRule::MaxLength(n))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Pattern`]. Regex is not
    /// validated at construction.
    #[must_use]
    pub fn pattern(p: impl Into<String>) -> Self {
        Self::Value(ValueRule::Pattern(p.into()))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Pattern`], validating
    /// the regex at construction. Returns the regex parse error on failure so
    /// callers can surface a specific diagnostic.
    pub fn try_pattern(p: impl Into<String>) -> Result<Self, regex::Error> {
        let p = p.into();
        regex::Regex::new(&p)?;
        Ok(Self::Value(ValueRule::Pattern(p)))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Min`] from an `i64`.
    #[must_use]
    pub fn min_value(n: i64) -> Self {
        Self::Value(ValueRule::Min(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Max`] from an `i64`.
    #[must_use]
    pub fn max_value(n: i64) -> Self {
        Self::Value(ValueRule::Max(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Min`] from an `f64`.
    /// Returns `None` if the value is NaN or infinite.
    #[must_use]
    pub fn min_value_f64(n: f64) -> Option<Self> {
        Some(Self::Value(ValueRule::Min(serde_json::Number::from_f64(
            n,
        )?)))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Max`] from an `f64`.
    /// Returns `None` if the value is NaN or infinite.
    #[must_use]
    pub fn max_value_f64(n: f64) -> Option<Self> {
        Some(Self::Value(ValueRule::Max(serde_json::Number::from_f64(
            n,
        )?)))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::GreaterThan`] from an `i64`.
    #[must_use]
    pub fn greater_than(n: i64) -> Self {
        Self::Value(ValueRule::GreaterThan(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::LessThan`] from an `i64`.
    #[must_use]
    pub fn less_than(n: i64) -> Self {
        Self::Value(ValueRule::LessThan(serde_json::Number::from(n)))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::OneOf`].
    #[must_use]
    pub fn one_of<V: Into<serde_json::Value>>(values: impl IntoIterator<Item = V>) -> Self {
        Self::Value(ValueRule::OneOf(
            values.into_iter().map(Into::into).collect(),
        ))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::MinItems`].
    #[must_use]
    pub fn min_items(n: usize) -> Self {
        Self::Value(ValueRule::MinItems(n))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::MaxItems`].
    #[must_use]
    pub fn max_items(n: usize) -> Self {
        Self::Value(ValueRule::MaxItems(n))
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Email`].
    #[must_use]
    pub fn email() -> Self {
        Self::Value(ValueRule::Email)
    }

    /// Creates a [`Rule::Value`] carrying a [`ValueRule::Url`].
    #[must_use]
    pub fn url() -> Self {
        Self::Value(ValueRule::Url)
    }

    /// Creates a [`Rule::Deferred`] carrying a [`DeferredRule::Custom`].
    #[must_use]
    pub fn custom(expression: impl Into<String>) -> Self {
        Self::Deferred(DeferredRule::Custom(expression.into()))
    }

    /// Creates a [`Rule::Deferred`] carrying a [`DeferredRule::UniqueBy`].
    /// Returns `None` if the path is invalid.
    #[must_use]
    pub fn unique_by(path: impl AsRef<str>) -> Option<Self> {
        Some(Self::Deferred(DeferredRule::UniqueBy(FieldPath::parse(
            path,
        )?)))
    }

    /// Creates a [`Rule::Logic`] carrying a [`Logic::All`].
    #[must_use]
    pub fn all(rules: impl IntoIterator<Item = Rule>) -> Self {
        Self::Logic(Box::new(Logic::All(rules.into_iter().collect())))
    }

    /// Creates a [`Rule::Logic`] carrying a [`Logic::Any`].
    #[must_use]
    pub fn any(rules: impl IntoIterator<Item = Rule>) -> Self {
        Self::Logic(Box::new(Logic::Any(rules.into_iter().collect())))
    }

    /// Creates a [`Rule::Logic`] carrying a [`Logic::Not`].
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "this is a rule constructor, not boolean negation"
    )]
    pub fn not(inner: Rule) -> Self {
        Self::Logic(Box::new(Logic::Not(inner)))
    }

    /// Wraps the rule with a custom error message.
    #[must_use]
    pub fn described(rule: Rule, message: impl Into<String>) -> Self {
        Self::Described(Box::new(rule), message.into())
    }

    /// Consumes `self` and wraps it in [`Rule::Described`] with a message.
    /// Sugar for building rules in method-chain style.
    #[must_use]
    pub fn with_message(self, message: impl Into<String>) -> Self {
        Self::described(self, message)
    }
}
