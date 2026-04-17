//! Constructor methods for [`Rule`]: shorthand builders for common rule
//! variants plus composition helpers (`all`, `any`, `not`, `with_message`).

use super::Rule;

impl Rule {
    // ── Shorthand constructors ──────────────────────────────────────────

    /// Creates a [`Pattern`](Self::Pattern) rule.
    ///
    /// The regex is **not** validated at construction time. If the pattern is
    /// invalid, [`validate_value`](Self::validate_value) will return an error
    /// with code `"invalid_pattern"`. Use [`try_pattern`](Self::try_pattern)
    /// when the pattern comes from user input.
    #[must_use]
    pub fn pattern(pattern: impl Into<String>) -> Self {
        Self::Pattern {
            pattern: pattern.into(),
            message: None,
        }
    }

    /// Creates a [`Pattern`](Self::Pattern) rule, validating the regex upfront.
    ///
    /// Returns `None` if the pattern is not a valid regular expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::Rule;
    ///
    /// assert!(Rule::try_pattern(r"^\d+$").is_some());
    /// assert!(Rule::try_pattern(r"[invalid").is_none());
    /// ```
    #[must_use]
    pub fn try_pattern(pattern: impl Into<String>) -> Option<Self> {
        let pattern = pattern.into();
        regex::Regex::new(&pattern).ok()?;
        Some(Self::Pattern {
            pattern,
            message: None,
        })
    }

    /// Creates a [`MinLength`](Self::MinLength) rule.
    #[must_use]
    pub fn min_length(min: usize) -> Self {
        Self::MinLength { min, message: None }
    }

    /// Creates a [`MaxLength`](Self::MaxLength) rule.
    #[must_use]
    pub fn max_length(max: usize) -> Self {
        Self::MaxLength { max, message: None }
    }

    /// Creates a [`Min`](Self::Min) rule from an `i64`.
    #[must_use]
    pub fn min_value(min: i64) -> Self {
        Self::Min {
            min: serde_json::Number::from(min),
            message: None,
        }
    }

    /// Creates a [`Max`](Self::Max) rule from an `i64`.
    #[must_use]
    pub fn max_value(max: i64) -> Self {
        Self::Max {
            max: serde_json::Number::from(max),
            message: None,
        }
    }

    /// Creates a [`Min`](Self::Min) rule from an `f64`.
    ///
    /// Returns `None` if `min` is NaN or infinite.
    #[must_use]
    pub fn min_value_f64(min: f64) -> Option<Self> {
        Some(Self::Min {
            min: serde_json::Number::from_f64(min)?,
            message: None,
        })
    }

    /// Creates a [`Max`](Self::Max) rule from an `f64`.
    ///
    /// Returns `None` if `max` is NaN or infinite.
    #[must_use]
    pub fn max_value_f64(max: f64) -> Option<Self> {
        Some(Self::Max {
            max: serde_json::Number::from_f64(max)?,
            message: None,
        })
    }

    /// Creates a [`GreaterThan`](Self::GreaterThan) rule from an `i64`.
    #[must_use]
    pub fn greater_than(min: i64) -> Self {
        Self::GreaterThan {
            min: serde_json::Number::from(min),
            message: None,
        }
    }

    /// Creates a [`LessThan`](Self::LessThan) rule from an `i64`.
    #[must_use]
    pub fn less_than(max: i64) -> Self {
        Self::LessThan {
            max: serde_json::Number::from(max),
            message: None,
        }
    }

    /// Creates a [`GreaterThan`](Self::GreaterThan) rule from an `f64`.
    ///
    /// Returns `None` if `min` is NaN or infinite.
    #[must_use]
    pub fn greater_than_f64(min: f64) -> Option<Self> {
        Some(Self::GreaterThan {
            min: serde_json::Number::from_f64(min)?,
            message: None,
        })
    }

    /// Creates a [`LessThan`](Self::LessThan) rule from an `f64`.
    ///
    /// Returns `None` if `max` is NaN or infinite.
    #[must_use]
    pub fn less_than_f64(max: f64) -> Option<Self> {
        Some(Self::LessThan {
            max: serde_json::Number::from_f64(max)?,
            message: None,
        })
    }

    /// Creates a [`OneOf`](Self::OneOf) rule.
    #[must_use]
    pub fn one_of<V: Into<serde_json::Value>>(values: impl IntoIterator<Item = V>) -> Self {
        Self::OneOf {
            values: values.into_iter().map(Into::into).collect(),
            message: None,
        }
    }

    /// Creates a [`MinItems`](Self::MinItems) rule.
    #[must_use]
    pub fn min_items(min: usize) -> Self {
        Self::MinItems { min, message: None }
    }

    /// Creates a [`MaxItems`](Self::MaxItems) rule.
    #[must_use]
    pub fn max_items(max: usize) -> Self {
        Self::MaxItems { max, message: None }
    }

    /// Creates an [`Email`](Self::Email) rule.
    #[must_use]
    pub fn email() -> Self {
        Self::Email { message: None }
    }

    /// Creates a [`Url`](Self::Url) rule.
    #[must_use]
    pub fn url() -> Self {
        Self::Url { message: None }
    }

    /// Creates a [`UniqueBy`](Self::UniqueBy) rule.
    #[must_use]
    pub fn unique_by(key: impl Into<String>) -> Self {
        Self::UniqueBy {
            key: key.into(),
            message: None,
        }
    }

    /// Creates a [`Custom`](Self::Custom) rule.
    #[must_use]
    pub fn custom(expression: impl Into<String>) -> Self {
        Self::Custom {
            expression: expression.into(),
            message: None,
        }
    }

    /// Attaches a custom error message to this rule.
    ///
    /// Applies to value-validation rules and deferred rules. **No-op** for
    /// context predicates (`Eq`, `Ne`, etc.) and logical combinators (`All`,
    /// `Any`, `Not`) — these variants do not carry a `message` field.
    #[must_use]
    pub fn with_message(self, msg: impl Into<String>) -> Self {
        let msg = Some(msg.into());
        match self {
            Self::Pattern { pattern, .. } => Self::Pattern {
                pattern,
                message: msg,
            },
            Self::MinLength { min, .. } => Self::MinLength { min, message: msg },
            Self::MaxLength { max, .. } => Self::MaxLength { max, message: msg },
            Self::Min { min, .. } => Self::Min { min, message: msg },
            Self::Max { max, .. } => Self::Max { max, message: msg },
            Self::GreaterThan { min, .. } => Self::GreaterThan { min, message: msg },
            Self::LessThan { max, .. } => Self::LessThan { max, message: msg },
            Self::OneOf { values, .. } => Self::OneOf {
                values,
                message: msg,
            },
            Self::MinItems { min, .. } => Self::MinItems { min, message: msg },
            Self::MaxItems { max, .. } => Self::MaxItems { max, message: msg },
            Self::Email { .. } => Self::Email { message: msg },
            Self::Url { .. } => Self::Url { message: msg },
            Self::UniqueBy { key, .. } => Self::UniqueBy { key, message: msg },
            Self::Custom { expression, .. } => Self::Custom {
                expression,
                message: msg,
            },
            // Predicate variants don't have messages
            other => other,
        }
    }

    /// Creates an [`All`](Self::All) rule (logical AND).
    #[must_use]
    pub fn all(rules: impl IntoIterator<Item = Self>) -> Self {
        Self::All {
            rules: rules.into_iter().collect(),
        }
    }

    /// Creates an [`Any`](Self::Any) rule (logical OR).
    #[must_use]
    pub fn any(rules: impl IntoIterator<Item = Self>) -> Self {
        Self::Any {
            rules: rules.into_iter().collect(),
        }
    }

    /// Creates a [`Not`](Self::Not) rule (logical negation).
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "this is a rule constructor, not boolean negation"
    )]
    pub fn not(inner: Self) -> Self {
        Self::Not {
            inner: Box::new(inner),
        }
    }
}
