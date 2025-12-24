//! Display condition system for conditional parameter visibility.
//!
//! This module provides a self-contained display condition system that evaluates
//! whether parameters should be shown based on the values of other parameters.
//!
//! # Overview
//!
//! The display system allows parameters to be conditionally shown or hidden based on:
//! - Equality comparisons (`Equals`, `NotEquals`)
//! - Null/set checks (`IsSet`, `IsNull`)
//! - Emptiness checks (`IsEmpty`, `IsNotEmpty`)
//! - Boolean checks (`IsTrue`, `IsFalse`)
//! - Numeric comparisons (`GreaterThan`, `LessThan`, `InRange`)
//! - String operations (`Contains`, `StartsWith`, `EndsWith`)
//! - Membership tests (`OneOf`)
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::core::display::DisplayCondition;
//! use nebula_value::Value;
//!
//! // Show field only when authentication type is "api_key"
//! let condition = DisplayCondition::Equals(Value::text("api_key"));
//! assert!(condition.evaluate(&Value::text("api_key")));
//! assert!(!condition.evaluate(&Value::text("oauth")));
//!
//! // Show field only when value is set (not null)
//! let condition = DisplayCondition::IsSet;
//! assert!(condition.evaluate(&Value::text("hello")));
//! assert!(!condition.evaluate(&Value::Null));
//!
//! // Show field only when count is in valid range
//! let condition = DisplayCondition::InRange { min: 1.0, max: 100.0 };
//! assert!(condition.evaluate(&Value::integer(50)));
//! assert!(!condition.evaluate(&Value::integer(150)));
//! ```

use crate::core::ParameterKey;
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A condition that determines whether a parameter should be displayed.
///
/// Display conditions evaluate a single value and return true if the parameter
/// should be shown, false otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayCondition {
    /// Value equals the specified value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::Equals(Value::text("api_key"));
    /// assert!(condition.evaluate(&Value::text("api_key")));
    /// assert!(!condition.evaluate(&Value::text("oauth")));
    /// ```
    Equals(Value),

    /// Value does not equal the specified value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::NotEquals(Value::text("disabled"));
    /// assert!(condition.evaluate(&Value::text("enabled")));
    /// assert!(!condition.evaluate(&Value::text("disabled")));
    /// ```
    NotEquals(Value),

    /// Value is not null.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::IsSet;
    /// assert!(condition.evaluate(&Value::text("hello")));
    /// assert!(condition.evaluate(&Value::integer(0)));
    /// assert!(!condition.evaluate(&Value::Null));
    /// ```
    IsSet,

    /// Value is null.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::IsNull;
    /// assert!(condition.evaluate(&Value::Null));
    /// assert!(!condition.evaluate(&Value::text("hello")));
    /// ```
    IsNull,

    /// Value is empty (empty string, empty array, or empty object).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::IsEmpty;
    /// assert!(condition.evaluate(&Value::text("")));
    /// assert!(condition.evaluate(&Value::array_empty()));
    /// assert!(!condition.evaluate(&Value::text("hello")));
    /// ```
    IsEmpty,

    /// Value is not empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::IsNotEmpty;
    /// assert!(condition.evaluate(&Value::text("hello")));
    /// assert!(!condition.evaluate(&Value::text("")));
    /// ```
    IsNotEmpty,

    /// Boolean value is true.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::IsTrue;
    /// assert!(condition.evaluate(&Value::boolean(true)));
    /// assert!(!condition.evaluate(&Value::boolean(false)));
    /// ```
    IsTrue,

    /// Boolean value is false.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::IsFalse;
    /// assert!(condition.evaluate(&Value::boolean(false)));
    /// assert!(!condition.evaluate(&Value::boolean(true)));
    /// ```
    IsFalse,

    /// Numeric value is greater than the specified threshold.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::GreaterThan(10.0);
    /// assert!(condition.evaluate(&Value::integer(15)));
    /// assert!(!condition.evaluate(&Value::integer(5)));
    /// ```
    GreaterThan(f64),

    /// Numeric value is less than the specified threshold.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::LessThan(100.0);
    /// assert!(condition.evaluate(&Value::integer(50)));
    /// assert!(!condition.evaluate(&Value::integer(150)));
    /// ```
    LessThan(f64),

    /// Numeric value is within the specified range (inclusive).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::InRange { min: 1.0, max: 100.0 };
    /// assert!(condition.evaluate(&Value::integer(50)));
    /// assert!(condition.evaluate(&Value::integer(1)));
    /// assert!(condition.evaluate(&Value::integer(100)));
    /// assert!(!condition.evaluate(&Value::integer(0)));
    /// assert!(!condition.evaluate(&Value::integer(101)));
    /// ```
    InRange {
        /// Minimum value (inclusive)
        min: f64,
        /// Maximum value (inclusive)
        max: f64,
    },

    /// String contains the specified substring.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::Contains("api".to_string());
    /// assert!(condition.evaluate(&Value::text("api_key")));
    /// assert!(!condition.evaluate(&Value::text("oauth")));
    /// ```
    Contains(String),

    /// String starts with the specified prefix.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::StartsWith("http".to_string());
    /// assert!(condition.evaluate(&Value::text("https://example.com")));
    /// assert!(!condition.evaluate(&Value::text("ftp://example.com")));
    /// ```
    StartsWith(String),

    /// String ends with the specified suffix.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::EndsWith(".json".to_string());
    /// assert!(condition.evaluate(&Value::text("config.json")));
    /// assert!(!condition.evaluate(&Value::text("config.yaml")));
    /// ```
    EndsWith(String),

    /// Value is one of the specified values.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::OneOf(vec![
    ///     Value::text("GET"),
    ///     Value::text("POST"),
    ///     Value::text("PUT"),
    /// ]);
    /// assert!(condition.evaluate(&Value::text("GET")));
    /// assert!(!condition.evaluate(&Value::text("DELETE")));
    /// ```
    OneOf(Vec<Value>),
}

impl DisplayCondition {
    /// Evaluate the condition against a value.
    ///
    /// Returns `true` if the condition is met, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// let condition = DisplayCondition::Equals(Value::text("test"));
    /// assert!(condition.evaluate(&Value::text("test")));
    /// assert!(!condition.evaluate(&Value::text("other")));
    /// ```
    #[must_use]
    pub fn evaluate(&self, value: &Value) -> bool {
        match self {
            Self::Equals(expected) => Self::values_equal(value, expected),
            Self::NotEquals(expected) => !Self::values_equal(value, expected),
            Self::IsSet => !value.is_null(),
            Self::IsNull => value.is_null(),
            Self::IsEmpty => Self::is_value_empty(value),
            Self::IsNotEmpty => !Self::is_value_empty(value),
            Self::IsTrue => value.as_boolean() == Some(true),
            Self::IsFalse => value.as_boolean() == Some(false),
            Self::GreaterThan(threshold) => {
                Self::get_numeric(value).map_or(false, |n| n > *threshold)
            }
            Self::LessThan(threshold) => Self::get_numeric(value).map_or(false, |n| n < *threshold),
            Self::InRange { min, max } => {
                Self::get_numeric(value).map_or(false, |n| n >= *min && n <= *max)
            }
            Self::Contains(substring) => {
                Self::get_string(value).map_or(false, |s| s.contains(substring))
            }
            Self::StartsWith(prefix) => {
                Self::get_string(value).map_or(false, |s| s.starts_with(prefix))
            }
            Self::EndsWith(suffix) => {
                Self::get_string(value).map_or(false, |s| s.ends_with(suffix))
            }
            Self::OneOf(values) => values.iter().any(|v| Self::values_equal(value, v)),
        }
    }

    /// Check if a value is empty.
    ///
    /// A value is considered empty if it is:
    /// - An empty string
    /// - An empty array
    /// - An empty object
    /// - Null (considered empty)
    ///
    /// All other values (numbers, booleans, non-empty strings/arrays/objects) are not empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// assert!(DisplayCondition::is_value_empty(&Value::text("")));
    /// assert!(DisplayCondition::is_value_empty(&Value::array_empty()));
    /// assert!(DisplayCondition::is_value_empty(&Value::object_empty()));
    /// assert!(DisplayCondition::is_value_empty(&Value::Null));
    /// assert!(!DisplayCondition::is_value_empty(&Value::text("hello")));
    /// assert!(!DisplayCondition::is_value_empty(&Value::integer(0)));
    /// ```
    #[must_use]
    pub fn is_value_empty(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::Text(t) => t.as_str().is_empty(),
            Value::Array(arr) => arr.is_empty(),
            Value::Object(obj) => obj.is_empty(),
            _ => false,
        }
    }

    /// Extract a numeric value as f64.
    ///
    /// Converts integers, floats, and decimals to f64. Returns `None` for non-numeric types.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// assert_eq!(DisplayCondition::get_numeric(&Value::integer(42)), Some(42.0));
    /// assert_eq!(DisplayCondition::get_numeric(&Value::float(3.14)), Some(3.14));
    /// assert_eq!(DisplayCondition::get_numeric(&Value::text("hello")), None);
    /// ```
    #[must_use]
    pub fn get_numeric(value: &Value) -> Option<f64> {
        value.as_float_lossy().map(|f| f.value())
    }

    /// Extract a string value as &str.
    ///
    /// Returns `None` for non-text values.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::DisplayCondition;
    /// use nebula_value::Value;
    ///
    /// assert_eq!(DisplayCondition::get_string(&Value::text("hello")), Some("hello"));
    /// assert_eq!(DisplayCondition::get_string(&Value::integer(42)), None);
    /// ```
    #[must_use]
    pub fn get_string(value: &Value) -> Option<&str> {
        value.as_str()
    }

    /// Compare two values for equality.
    ///
    /// This uses the PartialEq implementation from nebula_value::Value.
    /// For floating point numbers, this compares bit patterns (NaN == NaN is false).
    fn values_equal(a: &Value, b: &Value) -> bool {
        a == b
    }
}

/// Context containing resolved parameter values for display evaluation
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayContext {
    values: HashMap<ParameterKey, Value>,
}

impl DisplayContext {
    /// Create a new empty context
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a parameter value by key
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(&ParameterKey::from(key))
    }

    /// Builder pattern: add a value and return self
    #[must_use]
    pub fn with_value(mut self, key: impl Into<ParameterKey>, value: Value) -> Self {
        self.values.insert(key.into(), value);
        self
    }

    /// Insert a value
    pub fn insert(&mut self, key: impl Into<ParameterKey>, value: Value) {
        self.values.insert(key.into(), value);
    }

    /// Check if context contains a key
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(&ParameterKey::from(key))
    }

    /// Get all values as HashMap reference
    pub fn values(&self) -> &HashMap<ParameterKey, Value> {
        &self.values
    }
}

/// A display rule that checks a specific field against a condition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayRule {
    /// The parameter key to check
    pub field: ParameterKey,
    /// The condition to evaluate
    pub condition: DisplayCondition,
}

impl DisplayRule {
    /// Create a new display rule
    pub fn when(field: impl Into<ParameterKey>, condition: DisplayCondition) -> Self {
        Self {
            field: field.into(),
            condition,
        }
    }

    /// Evaluate this rule against a context
    pub fn evaluate(&self, ctx: &DisplayContext) -> bool {
        match ctx.get(self.field.as_str()) {
            Some(value) => self.condition.evaluate(value),
            None => false, // Missing field = condition not met
        }
    }

    /// Get the field this rule depends on
    pub fn dependency(&self) -> &ParameterKey {
        &self.field
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_value::Value;

    #[test]
    fn test_condition_equals() {
        let condition = DisplayCondition::Equals(Value::text("api_key"));
        assert!(condition.evaluate(&Value::text("api_key")));
        assert!(!condition.evaluate(&Value::text("oauth")));
        assert!(!condition.evaluate(&Value::Null));
    }

    #[test]
    fn test_condition_equals_numbers() {
        let condition = DisplayCondition::Equals(Value::integer(42));
        assert!(condition.evaluate(&Value::integer(42)));
        assert!(!condition.evaluate(&Value::integer(43)));
        assert!(!condition.evaluate(&Value::float(42.0)));
    }

    #[test]
    fn test_condition_not_equals() {
        let condition = DisplayCondition::NotEquals(Value::text("disabled"));
        assert!(condition.evaluate(&Value::text("enabled")));
        assert!(!condition.evaluate(&Value::text("disabled")));
        assert!(condition.evaluate(&Value::Null));
    }

    #[test]
    fn test_condition_is_set() {
        let condition = DisplayCondition::IsSet;
        assert!(condition.evaluate(&Value::text("hello")));
        assert!(condition.evaluate(&Value::integer(0)));
        assert!(condition.evaluate(&Value::boolean(false)));
        assert!(condition.evaluate(&Value::text("")));
        assert!(!condition.evaluate(&Value::Null));
    }

    #[test]
    fn test_condition_is_null() {
        let condition = DisplayCondition::IsNull;
        assert!(condition.evaluate(&Value::Null));
        assert!(!condition.evaluate(&Value::text("hello")));
        assert!(!condition.evaluate(&Value::integer(0)));
    }

    #[test]
    fn test_condition_is_empty() {
        let condition = DisplayCondition::IsEmpty;
        assert!(condition.evaluate(&Value::text("")));
        assert!(condition.evaluate(&Value::array_empty()));
        assert!(condition.evaluate(&Value::object_empty()));
        assert!(condition.evaluate(&Value::Null));
        assert!(!condition.evaluate(&Value::text("hello")));
        assert!(!condition.evaluate(&Value::integer(0)));
    }

    #[test]
    fn test_condition_is_not_empty() {
        let condition = DisplayCondition::IsNotEmpty;
        assert!(condition.evaluate(&Value::text("hello")));
        assert!(condition.evaluate(&Value::integer(42)));
        assert!(!condition.evaluate(&Value::text("")));
        assert!(!condition.evaluate(&Value::array_empty()));
        assert!(!condition.evaluate(&Value::Null));
    }

    #[test]
    fn test_condition_is_true() {
        let condition = DisplayCondition::IsTrue;
        assert!(condition.evaluate(&Value::boolean(true)));
        assert!(!condition.evaluate(&Value::boolean(false)));
        assert!(!condition.evaluate(&Value::integer(1)));
        assert!(!condition.evaluate(&Value::Null));
    }

    #[test]
    fn test_condition_is_false() {
        let condition = DisplayCondition::IsFalse;
        assert!(condition.evaluate(&Value::boolean(false)));
        assert!(!condition.evaluate(&Value::boolean(true)));
        assert!(!condition.evaluate(&Value::integer(0)));
        assert!(!condition.evaluate(&Value::Null));
    }

    #[test]
    fn test_condition_greater_than() {
        let condition = DisplayCondition::GreaterThan(10.0);
        assert!(condition.evaluate(&Value::integer(15)));
        assert!(condition.evaluate(&Value::float(10.5)));
        assert!(!condition.evaluate(&Value::integer(10)));
        assert!(!condition.evaluate(&Value::integer(5)));
        assert!(!condition.evaluate(&Value::text("hello")));
    }

    #[test]
    fn test_condition_less_than() {
        let condition = DisplayCondition::LessThan(100.0);
        assert!(condition.evaluate(&Value::integer(50)));
        assert!(condition.evaluate(&Value::float(99.9)));
        assert!(!condition.evaluate(&Value::integer(100)));
        assert!(!condition.evaluate(&Value::integer(150)));
        assert!(!condition.evaluate(&Value::text("hello")));
    }

    #[test]
    fn test_condition_in_range() {
        let condition = DisplayCondition::InRange {
            min: 1.0,
            max: 100.0,
        };
        assert!(condition.evaluate(&Value::integer(50)));
        assert!(condition.evaluate(&Value::integer(1)));
        assert!(condition.evaluate(&Value::integer(100)));
        assert!(condition.evaluate(&Value::float(50.5)));
        assert!(!condition.evaluate(&Value::integer(0)));
        assert!(!condition.evaluate(&Value::integer(101)));
        assert!(!condition.evaluate(&Value::text("50")));
    }

    #[test]
    fn test_condition_contains() {
        let condition = DisplayCondition::Contains("api".to_string());
        assert!(condition.evaluate(&Value::text("api_key")));
        assert!(condition.evaluate(&Value::text("my_api")));
        assert!(condition.evaluate(&Value::text("api")));
        assert!(!condition.evaluate(&Value::text("oauth")));
        assert!(!condition.evaluate(&Value::integer(42)));
    }

    #[test]
    fn test_condition_starts_with() {
        let condition = DisplayCondition::StartsWith("http".to_string());
        assert!(condition.evaluate(&Value::text("https://example.com")));
        assert!(condition.evaluate(&Value::text("http://example.com")));
        assert!(!condition.evaluate(&Value::text("ftp://example.com")));
        assert!(!condition.evaluate(&Value::text("example.com")));
    }

    #[test]
    fn test_condition_ends_with() {
        let condition = DisplayCondition::EndsWith(".json".to_string());
        assert!(condition.evaluate(&Value::text("config.json")));
        assert!(condition.evaluate(&Value::text("data.json")));
        assert!(!condition.evaluate(&Value::text("config.yaml")));
        assert!(!condition.evaluate(&Value::text("json")));
    }

    #[test]
    fn test_condition_one_of() {
        let condition = DisplayCondition::OneOf(vec![
            Value::text("GET"),
            Value::text("POST"),
            Value::text("PUT"),
        ]);
        assert!(condition.evaluate(&Value::text("GET")));
        assert!(condition.evaluate(&Value::text("POST")));
        assert!(condition.evaluate(&Value::text("PUT")));
        assert!(!condition.evaluate(&Value::text("DELETE")));
        assert!(!condition.evaluate(&Value::text("PATCH")));
    }

    #[test]
    fn test_condition_one_of_numbers() {
        let condition = DisplayCondition::OneOf(vec![
            Value::integer(1),
            Value::integer(2),
            Value::integer(3),
        ]);
        assert!(condition.evaluate(&Value::integer(1)));
        assert!(condition.evaluate(&Value::integer(2)));
        assert!(condition.evaluate(&Value::integer(3)));
        assert!(!condition.evaluate(&Value::integer(4)));
    }

    #[test]
    fn test_helper_is_value_empty() {
        assert!(DisplayCondition::is_value_empty(&Value::text("")));
        assert!(DisplayCondition::is_value_empty(&Value::array_empty()));
        assert!(DisplayCondition::is_value_empty(&Value::object_empty()));
        assert!(DisplayCondition::is_value_empty(&Value::Null));
        assert!(!DisplayCondition::is_value_empty(&Value::text("hello")));
        assert!(!DisplayCondition::is_value_empty(&Value::integer(0)));
        assert!(!DisplayCondition::is_value_empty(&Value::boolean(false)));
    }

    #[test]
    fn test_helper_get_numeric() {
        assert_eq!(
            DisplayCondition::get_numeric(&Value::integer(42)),
            Some(42.0)
        );
        assert_eq!(
            DisplayCondition::get_numeric(&Value::float(3.14)),
            Some(3.14)
        );
        assert_eq!(DisplayCondition::get_numeric(&Value::text("hello")), None);
        assert_eq!(DisplayCondition::get_numeric(&Value::Null), None);
    }

    #[test]
    fn test_helper_get_string() {
        assert_eq!(
            DisplayCondition::get_string(&Value::text("hello")),
            Some("hello")
        );
        assert_eq!(DisplayCondition::get_string(&Value::text("")), Some(""));
        assert_eq!(DisplayCondition::get_string(&Value::integer(42)), None);
        assert_eq!(DisplayCondition::get_string(&Value::Null), None);
    }

    #[test]
    fn test_serialization() {
        let condition = DisplayCondition::Equals(Value::text("test"));
        let json = serde_json::to_string(&condition).expect("Failed to serialize");
        let deserialized: DisplayCondition =
            serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(condition, deserialized);
    }

    #[test]
    fn test_clone() {
        let condition = DisplayCondition::Equals(Value::text("test"));
        let cloned = condition.clone();
        assert_eq!(condition, cloned);
    }

    #[test]
    fn test_display_rule_single() {
        let rule = DisplayRule::when(
            "auth_type",
            DisplayCondition::Equals(Value::text("api_key")),
        );

        let ctx = DisplayContext::new().with_value("auth_type", Value::text("api_key"));

        assert!(rule.evaluate(&ctx));
    }

    #[test]
    fn test_display_rule_missing_field() {
        let rule = DisplayRule::when(
            "auth_type",
            DisplayCondition::Equals(Value::text("api_key")),
        );

        let ctx = DisplayContext::new(); // No auth_type

        assert!(!rule.evaluate(&ctx)); // Missing field = condition not met
    }

    #[test]
    fn test_display_context_builder() {
        let ctx = DisplayContext::new()
            .with_value("a", Value::integer(1))
            .with_value("b", Value::text("hello"));

        assert_eq!(ctx.get("a"), Some(&Value::integer(1)));
        assert_eq!(ctx.get("b"), Some(&Value::text("hello")));
        assert_eq!(ctx.get("c"), None);
    }
}
