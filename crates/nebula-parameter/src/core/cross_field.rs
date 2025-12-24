//! Cross-field validation rules.
//!
//! Rules for comparing the current field against other fields in the form.

use nebula_core::ParameterKey;
use serde::{Deserialize, Serialize};

/// Rule comparing current field to another field.
///
/// These rules validate relationships between fields, such as "confirm password
/// must equal password" or "end date must be after start date".
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::core::CrossFieldRule;
/// use nebula_core::ParameterKey;
///
/// // Password confirmation must match password
/// let rule = CrossFieldRule::EqualsField(ParameterKey::new("password").unwrap());
///
/// // End date must be after start date
/// let rule = CrossFieldRule::AfterField(ParameterKey::new("start_date").unwrap());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrossFieldRule {
    // === Equality ===
    /// Current field must equal the referenced field.
    EqualsField(ParameterKey),

    /// Current field must not equal the referenced field.
    NotEqualsField(ParameterKey),

    // === Numeric comparisons ===
    /// Current field must be greater than the referenced field.
    GreaterThanField(ParameterKey),

    /// Current field must be greater than or equal to the referenced field.
    GreaterOrEqualField(ParameterKey),

    /// Current field must be less than the referenced field.
    LessThanField(ParameterKey),

    /// Current field must be less than or equal to the referenced field.
    LessOrEqualField(ParameterKey),

    // === String operations ===
    /// Current field must contain the value of the referenced field as substring.
    ContainsField(ParameterKey),

    /// Current field's value must be contained in the referenced field.
    ContainedInField(ParameterKey),

    // === Temporal comparisons ===
    /// Current field's date/time must be before the referenced field.
    BeforeField(ParameterKey),

    /// Current field's date/time must be after the referenced field.
    AfterField(ParameterKey),
}

impl CrossFieldRule {
    /// Get the field this rule references.
    #[must_use]
    pub fn referenced_field(&self) -> &ParameterKey {
        match self {
            Self::EqualsField(f)
            | Self::NotEqualsField(f)
            | Self::GreaterThanField(f)
            | Self::GreaterOrEqualField(f)
            | Self::LessThanField(f)
            | Self::LessOrEqualField(f)
            | Self::ContainsField(f)
            | Self::ContainedInField(f)
            | Self::BeforeField(f)
            | Self::AfterField(f) => f,
        }
    }

    /// Create an equals rule.
    pub fn equals(field: impl Into<ParameterKey>) -> Self {
        Self::EqualsField(field.into())
    }

    /// Create a not-equals rule.
    pub fn not_equals(field: impl Into<ParameterKey>) -> Self {
        Self::NotEqualsField(field.into())
    }

    /// Create a greater-than rule.
    pub fn greater_than(field: impl Into<ParameterKey>) -> Self {
        Self::GreaterThanField(field.into())
    }

    /// Create a greater-or-equal rule.
    pub fn greater_or_equal(field: impl Into<ParameterKey>) -> Self {
        Self::GreaterOrEqualField(field.into())
    }

    /// Create a less-than rule.
    pub fn less_than(field: impl Into<ParameterKey>) -> Self {
        Self::LessThanField(field.into())
    }

    /// Create a less-or-equal rule.
    pub fn less_or_equal(field: impl Into<ParameterKey>) -> Self {
        Self::LessOrEqualField(field.into())
    }

    /// Create a before rule (for dates/times).
    pub fn before(field: impl Into<ParameterKey>) -> Self {
        Self::BeforeField(field.into())
    }

    /// Create an after rule (for dates/times).
    pub fn after(field: impl Into<ParameterKey>) -> Self {
        Self::AfterField(field.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ParameterKey {
        ParameterKey::new(s).unwrap()
    }

    #[test]
    fn test_referenced_field() {
        let rule = CrossFieldRule::EqualsField(key("password"));
        assert_eq!(rule.referenced_field(), &key("password"));

        let rule = CrossFieldRule::AfterField(key("start_date"));
        assert_eq!(rule.referenced_field(), &key("start_date"));
    }

    #[test]
    fn test_constructor_methods() {
        assert_eq!(
            CrossFieldRule::equals(key("foo")),
            CrossFieldRule::EqualsField(key("foo"))
        );
        assert_eq!(
            CrossFieldRule::not_equals(key("bar")),
            CrossFieldRule::NotEqualsField(key("bar"))
        );
        assert_eq!(
            CrossFieldRule::greater_than(key("min")),
            CrossFieldRule::GreaterThanField(key("min"))
        );
        assert_eq!(
            CrossFieldRule::before(key("end")),
            CrossFieldRule::BeforeField(key("end"))
        );
        assert_eq!(
            CrossFieldRule::after(key("start")),
            CrossFieldRule::AfterField(key("start"))
        );
    }

    #[test]
    fn test_serialization() {
        let rule = CrossFieldRule::EqualsField(key("password"));
        let json = serde_json::to_string(&rule).expect("serialize");
        let parsed: CrossFieldRule = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(rule, parsed);
    }
}
