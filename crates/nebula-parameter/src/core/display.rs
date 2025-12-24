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

use crate::core::values::ParameterValues;
use nebula_core::ParameterKey;
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

    /// Field has passed validation (is valid).
    ///
    /// This condition checks the validation state of a field, not its value.
    /// It returns true if the field has been validated and passed validation.
    ///
    /// Note: This condition is evaluated against the `DisplayContext`'s validation
    /// state, not the value itself. When using `evaluate()` directly on a value,
    /// this will always return false. Use `DisplayRule::evaluate()` with a context
    /// that has validation state set.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::{DisplayCondition, DisplayContext, DisplayRule};
    /// use nebula_core::ParameterKey;
    ///
    /// let rule = DisplayRule::when(
    ///     ParameterKey::new("password").unwrap(),
    ///     DisplayCondition::IsValid,
    /// );
    ///
    /// let ctx = DisplayContext::new()
    ///     .with_validation(ParameterKey::new("password").unwrap(), true);
    ///
    /// assert!(rule.evaluate(&ctx));
    /// ```
    IsValid,

    /// Field has failed validation (is invalid).
    ///
    /// This condition checks the validation state of a field, not its value.
    /// It returns true if the field has been validated and failed validation.
    ///
    /// Note: This condition is evaluated against the `DisplayContext`'s validation
    /// state, not the value itself. When using `evaluate()` directly on a value,
    /// this will always return false. Use `DisplayRule::evaluate()` with a context
    /// that has validation state set.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::display::{DisplayCondition, DisplayContext, DisplayRule};
    /// use nebula_core::ParameterKey;
    ///
    /// let rule = DisplayRule::when(
    ///     ParameterKey::new("email").unwrap(),
    ///     DisplayCondition::IsInvalid,
    /// );
    ///
    /// let ctx = DisplayContext::new()
    ///     .with_validation(ParameterKey::new("email").unwrap(), false);
    ///
    /// assert!(rule.evaluate(&ctx));
    /// ```
    IsInvalid,
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
                Self::get_numeric(value).is_some_and(|n| n > *threshold)
            }
            Self::LessThan(threshold) => Self::get_numeric(value).is_some_and(|n| n < *threshold),
            Self::InRange { min, max } => {
                Self::get_numeric(value).is_some_and(|n| n >= *min && n <= *max)
            }
            Self::Contains(substring) => {
                Self::get_string(value).is_some_and(|s| s.contains(substring))
            }
            Self::StartsWith(prefix) => {
                Self::get_string(value).is_some_and(|s| s.starts_with(prefix))
            }
            Self::EndsWith(suffix) => Self::get_string(value).is_some_and(|s| s.ends_with(suffix)),
            Self::OneOf(values) => values.iter().any(|v| Self::values_equal(value, v)),
            // IsValid/IsInvalid cannot be evaluated against a value alone,
            // they require the DisplayContext. Always return false here.
            Self::IsValid | Self::IsInvalid => false,
        }
    }

    /// Check if this condition requires validation state (not just value).
    ///
    /// Returns `true` for `IsValid` and `IsInvalid` conditions.
    #[must_use]
    pub fn requires_validation_state(&self) -> bool {
        matches!(self, Self::IsValid | Self::IsInvalid)
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

/// Context containing resolved parameter values and validation state for display evaluation
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayContext {
    /// Parameter values using the standard ParameterValues storage
    values: ParameterValues,
    /// Validation state for each parameter (true = valid, false = invalid)
    validation: HashMap<ParameterKey, bool>,
}

impl DisplayContext {
    /// Create a new empty context
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create from existing ParameterValues
    #[must_use]
    pub fn from_values(values: ParameterValues) -> Self {
        Self {
            values,
            validation: HashMap::new(),
        }
    }

    /// Get a parameter value by key
    pub fn get(&self, key: &str) -> Option<&Value> {
        ParameterKey::new(key).ok().and_then(|k| self.values.get(k))
    }

    /// Get validation state for a parameter
    ///
    /// Returns `Some(true)` if valid, `Some(false)` if invalid,
    /// `None` if validation state is not set (not yet validated).
    pub fn get_validation(&self, key: &str) -> Option<bool> {
        ParameterKey::new(key)
            .ok()
            .and_then(|k| self.validation.get(&k).copied())
    }

    /// Check if a parameter is valid
    ///
    /// Returns `true` only if the parameter has been validated and passed.
    /// Returns `false` if invalid or not yet validated.
    pub fn is_valid(&self, key: &str) -> bool {
        self.get_validation(key) == Some(true)
    }

    /// Check if a parameter is invalid
    ///
    /// Returns `true` only if the parameter has been validated and failed.
    /// Returns `false` if valid or not yet validated.
    pub fn is_invalid(&self, key: &str) -> bool {
        self.get_validation(key) == Some(false)
    }

    /// Builder pattern: add a value and return self
    #[must_use]
    pub fn with_value(mut self, key: impl Into<ParameterKey>, value: Value) -> Self {
        self.values.set(key, value);
        self
    }

    /// Builder pattern: add validation state and return self
    #[must_use]
    pub fn with_validation(mut self, key: impl Into<ParameterKey>, is_valid: bool) -> Self {
        self.validation.insert(key.into(), is_valid);
        self
    }

    /// Builder pattern: set all values from ParameterValues
    #[must_use]
    pub fn with_values(mut self, values: ParameterValues) -> Self {
        self.values = values;
        self
    }

    /// Insert a value
    pub fn insert(&mut self, key: impl Into<ParameterKey>, value: Value) {
        self.values.set(key, value);
    }

    /// Set validation state for a parameter
    pub fn set_validation(&mut self, key: impl Into<ParameterKey>, is_valid: bool) {
        self.validation.insert(key.into(), is_valid);
    }

    /// Mark a parameter as valid
    pub fn mark_valid(&mut self, key: impl Into<ParameterKey>) {
        self.set_validation(key, true);
    }

    /// Mark a parameter as invalid
    pub fn mark_invalid(&mut self, key: impl Into<ParameterKey>) {
        self.set_validation(key, false);
    }

    /// Clear validation state for a parameter
    pub fn clear_validation(&mut self, key: &str) {
        if let Ok(k) = ParameterKey::new(key) {
            self.validation.remove(&k);
        }
    }

    /// Check if context contains a key
    pub fn contains(&self, key: &str) -> bool {
        ParameterKey::new(key)
            .ok()
            .map(|k| self.values.contains(k))
            .unwrap_or(false)
    }

    /// Get all values as ParameterValues reference
    pub fn values(&self) -> &ParameterValues {
        &self.values
    }

    /// Get mutable access to values
    pub fn values_mut(&mut self) -> &mut ParameterValues {
        &mut self.values
    }

    /// Get all validation states as HashMap reference
    pub fn validations(&self) -> &HashMap<ParameterKey, bool> {
        &self.validation
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
        // Handle validation-based conditions specially
        match &self.condition {
            DisplayCondition::IsValid => ctx.is_valid(self.field.as_str()),
            DisplayCondition::IsInvalid => ctx.is_invalid(self.field.as_str()),
            // For value-based conditions, get the value and evaluate
            _ => match ctx.get(self.field.as_str()) {
                Some(value) => self.condition.evaluate(value),
                None => false, // Missing field = condition not met
            },
        }
    }

    /// Get the field this rule depends on
    pub fn dependency(&self) -> &ParameterKey {
        &self.field
    }
}

/// A set of display rules combined with logical operators
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplayRuleSet {
    /// A single rule
    Single(DisplayRule),
    /// All rules must pass (AND)
    All(Vec<DisplayRuleSet>),
    /// Any rule must pass (OR)
    Any(Vec<DisplayRuleSet>),
    /// Rule must not pass (NOT)
    Not(Box<DisplayRuleSet>),
}

impl DisplayRuleSet {
    /// Create from a single rule
    pub fn single(rule: DisplayRule) -> Self {
        DisplayRuleSet::Single(rule)
    }

    /// Create an ALL ruleset (AND)
    pub fn all(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        DisplayRuleSet::All(rules.into_iter().map(Into::into).collect())
    }

    /// Create an ANY ruleset (OR)
    pub fn any(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        DisplayRuleSet::Any(rules.into_iter().map(Into::into).collect())
    }

    /// Create a NOT ruleset
    pub fn not(rule: impl Into<DisplayRuleSet>) -> Self {
        DisplayRuleSet::Not(Box::new(rule.into()))
    }

    /// Evaluate this ruleset against a context
    pub fn evaluate(&self, ctx: &DisplayContext) -> bool {
        match self {
            DisplayRuleSet::Single(rule) => rule.evaluate(ctx),
            DisplayRuleSet::All(rules) => rules.iter().all(|r| r.evaluate(ctx)),
            DisplayRuleSet::Any(rules) => rules.iter().any(|r| r.evaluate(ctx)),
            DisplayRuleSet::Not(rule) => !rule.evaluate(ctx),
        }
    }

    /// Get all parameter dependencies from this ruleset
    pub fn dependencies(&self) -> Vec<ParameterKey> {
        let mut deps = Vec::new();
        self.collect_dependencies(&mut deps);
        deps.sort();
        deps.dedup();
        deps
    }

    fn collect_dependencies(&self, deps: &mut Vec<ParameterKey>) {
        match self {
            DisplayRuleSet::Single(rule) => {
                deps.push(rule.field.clone());
            }
            DisplayRuleSet::All(rules) | DisplayRuleSet::Any(rules) => {
                for rule in rules {
                    rule.collect_dependencies(deps);
                }
            }
            DisplayRuleSet::Not(rule) => {
                rule.collect_dependencies(deps);
            }
        }
    }
}

impl From<DisplayRule> for DisplayRuleSet {
    fn from(rule: DisplayRule) -> Self {
        DisplayRuleSet::Single(rule)
    }
}

/// Configuration determining when a parameter should be displayed
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterDisplay {
    /// Conditions that must be met to show the parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    show_when: Option<DisplayRuleSet>,
    /// Conditions that cause the parameter to be hidden (takes priority)
    #[serde(skip_serializing_if = "Option::is_none")]
    hide_when: Option<DisplayRuleSet>,
}

impl ParameterDisplay {
    /// Create a new empty display configuration
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a show condition
    #[must_use]
    pub fn show_when(mut self, rule: impl Into<DisplayRuleSet>) -> Self {
        let ruleset = rule.into();
        self.show_when = Some(match self.show_when.take() {
            Some(existing) => DisplayRuleSet::All(vec![existing, ruleset]),
            None => ruleset,
        });
        self
    }

    /// Add a hide condition
    #[must_use]
    pub fn hide_when(mut self, rule: impl Into<DisplayRuleSet>) -> Self {
        let ruleset = rule.into();
        self.hide_when = Some(match self.hide_when.take() {
            Some(existing) => DisplayRuleSet::Any(vec![existing, ruleset]),
            None => ruleset,
        });
        self
    }

    /// Check if parameter should be displayed
    pub fn should_display(&self, ctx: &DisplayContext) -> bool {
        // Priority: hide_when is checked first
        if self
            .hide_when
            .as_ref()
            .is_some_and(|rules| rules.evaluate(ctx))
        {
            return false;
        }

        // Then check show_when (if no show_when, default to show)
        self.show_when
            .as_ref()
            .is_none_or(|rules| rules.evaluate(ctx))
    }

    /// Check if this display has no conditions
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.show_when.is_none() && self.hide_when.is_none()
    }

    /// Get all parameter dependencies
    #[must_use]
    pub fn dependencies(&self) -> Vec<ParameterKey> {
        let mut deps = Vec::new();

        if let Some(show) = &self.show_when {
            deps.extend(show.dependencies());
        }

        if let Some(hide) = &self.hide_when {
            deps.extend(hide.dependencies());
        }

        deps.sort();
        deps.dedup();
        deps
    }

    /// Convenience: show when field equals value
    #[must_use]
    pub fn show_when_equals(self, field: impl Into<ParameterKey>, value: Value) -> Self {
        self.show_when(DisplayRule::when(field, DisplayCondition::Equals(value)))
    }

    /// Convenience: show when field is true
    #[must_use]
    pub fn show_when_true(self, field: impl Into<ParameterKey>) -> Self {
        self.show_when(DisplayRule::when(field, DisplayCondition::IsTrue))
    }

    /// Convenience: hide when field equals value
    #[must_use]
    pub fn hide_when_equals(self, field: impl Into<ParameterKey>, value: Value) -> Self {
        self.hide_when(DisplayRule::when(field, DisplayCondition::Equals(value)))
    }

    /// Convenience: hide when field is true
    #[must_use]
    pub fn hide_when_true(self, field: impl Into<ParameterKey>) -> Self {
        self.hide_when(DisplayRule::when(field, DisplayCondition::IsTrue))
    }

    /// Convenience: show when field is valid (passed validation)
    #[must_use]
    pub fn show_when_valid(self, field: impl Into<ParameterKey>) -> Self {
        self.show_when(DisplayRule::when(field, DisplayCondition::IsValid))
    }

    /// Convenience: show when field is invalid (failed validation)
    #[must_use]
    pub fn show_when_invalid(self, field: impl Into<ParameterKey>) -> Self {
        self.show_when(DisplayRule::when(field, DisplayCondition::IsInvalid))
    }

    /// Convenience: hide when field is valid
    #[must_use]
    pub fn hide_when_valid(self, field: impl Into<ParameterKey>) -> Self {
        self.hide_when(DisplayRule::when(field, DisplayCondition::IsValid))
    }

    /// Convenience: hide when field is invalid
    #[must_use]
    pub fn hide_when_invalid(self, field: impl Into<ParameterKey>) -> Self {
        self.hide_when(DisplayRule::when(field, DisplayCondition::IsInvalid))
    }
}

/// Error type for display validation
#[derive(Debug, Clone, thiserror::Error)]
#[error("Display condition not met: {message}")]
pub struct ParameterDisplayError {
    /// Error message
    pub message: String,
}

impl ParameterDisplayError {
    /// Create a new error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
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
            ParameterKey::new("auth_type").unwrap(),
            DisplayCondition::Equals(Value::text("api_key")),
        );

        let ctx = DisplayContext::new().with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        );

        assert!(rule.evaluate(&ctx));
    }

    #[test]
    fn test_display_rule_missing_field() {
        let rule = DisplayRule::when(
            ParameterKey::new("auth_type").unwrap(),
            DisplayCondition::Equals(Value::text("api_key")),
        );

        let ctx = DisplayContext::new(); // No auth_type

        assert!(!rule.evaluate(&ctx)); // Missing field = condition not met
    }

    #[test]
    fn test_display_context_builder() {
        let ctx = DisplayContext::new()
            .with_value(ParameterKey::new("a").unwrap(), Value::integer(1))
            .with_value(ParameterKey::new("b").unwrap(), Value::text("hello"));

        assert_eq!(ctx.get("a"), Some(&Value::integer(1)));
        assert_eq!(ctx.get("b"), Some(&Value::text("hello")));
        assert_eq!(ctx.get("c"), None);
    }

    #[test]
    fn test_ruleset_and() {
        let ruleset = DisplayRuleSet::all([
            DisplayRule::when(
                ParameterKey::new("enabled").unwrap(),
                DisplayCondition::IsTrue,
            ),
            DisplayRule::when(
                ParameterKey::new("level").unwrap(),
                DisplayCondition::GreaterThan(10.0),
            ),
        ]);

        let ctx_pass = DisplayContext::new()
            .with_value(ParameterKey::new("enabled").unwrap(), Value::boolean(true))
            .with_value(ParameterKey::new("level").unwrap(), Value::integer(15));

        let ctx_fail = DisplayContext::new()
            .with_value(ParameterKey::new("enabled").unwrap(), Value::boolean(true))
            .with_value(ParameterKey::new("level").unwrap(), Value::integer(5));

        assert!(ruleset.evaluate(&ctx_pass));
        assert!(!ruleset.evaluate(&ctx_fail));
    }

    #[test]
    fn test_ruleset_or() {
        let ruleset = DisplayRuleSet::any([
            DisplayRule::when(
                ParameterKey::new("role").unwrap(),
                DisplayCondition::Equals(Value::text("admin")),
            ),
            DisplayRule::when(
                ParameterKey::new("superuser").unwrap(),
                DisplayCondition::IsTrue,
            ),
        ]);

        let ctx_admin = DisplayContext::new()
            .with_value(ParameterKey::new("role").unwrap(), Value::text("admin"));

        let ctx_superuser = DisplayContext::new().with_value(
            ParameterKey::new("superuser").unwrap(),
            Value::boolean(true),
        );

        let ctx_neither = DisplayContext::new()
            .with_value(ParameterKey::new("role").unwrap(), Value::text("user"));

        assert!(ruleset.evaluate(&ctx_admin));
        assert!(ruleset.evaluate(&ctx_superuser));
        assert!(!ruleset.evaluate(&ctx_neither));
    }

    #[test]
    fn test_ruleset_not() {
        let ruleset = DisplayRuleSet::not(DisplayRule::when(
            ParameterKey::new("disabled").unwrap(),
            DisplayCondition::IsTrue,
        ));

        let ctx_enabled = DisplayContext::new().with_value(
            ParameterKey::new("disabled").unwrap(),
            Value::boolean(false),
        );

        let ctx_disabled = DisplayContext::new()
            .with_value(ParameterKey::new("disabled").unwrap(), Value::boolean(true));

        assert!(ruleset.evaluate(&ctx_enabled));
        assert!(!ruleset.evaluate(&ctx_disabled));
    }

    #[test]
    fn test_ruleset_dependencies() {
        let ruleset = DisplayRuleSet::all([
            DisplayRule::when(ParameterKey::new("a").unwrap(), DisplayCondition::IsTrue),
            DisplayRule::when(ParameterKey::new("b").unwrap(), DisplayCondition::IsTrue),
        ]);

        let deps = ruleset.dependencies();
        assert!(deps.contains(&ParameterKey::new("a").unwrap()));
        assert!(deps.contains(&ParameterKey::new("b").unwrap()));
    }

    #[test]
    fn test_parameter_display_show_when() {
        let display = ParameterDisplay::new().show_when(DisplayRule::when(
            ParameterKey::new("auth_type").unwrap(),
            DisplayCondition::Equals(Value::text("api_key")),
        ));

        let ctx_show = DisplayContext::new().with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        );

        let ctx_hide = DisplayContext::new().with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("oauth"),
        );

        assert!(display.should_display(&ctx_show));
        assert!(!display.should_display(&ctx_hide));
    }

    #[test]
    fn test_parameter_display_hide_when() {
        let display = ParameterDisplay::new().hide_when(DisplayRule::when(
            ParameterKey::new("disabled").unwrap(),
            DisplayCondition::IsTrue,
        ));

        let ctx_show = DisplayContext::new().with_value(
            ParameterKey::new("disabled").unwrap(),
            Value::boolean(false),
        );

        let ctx_hide = DisplayContext::new()
            .with_value(ParameterKey::new("disabled").unwrap(), Value::boolean(true));

        assert!(display.should_display(&ctx_show));
        assert!(!display.should_display(&ctx_hide));
    }

    #[test]
    fn test_parameter_display_hide_takes_priority() {
        // hide_when is checked first
        let display = ParameterDisplay::new()
            .show_when(DisplayRule::when(
                ParameterKey::new("enabled").unwrap(),
                DisplayCondition::IsTrue,
            ))
            .hide_when(DisplayRule::when(
                ParameterKey::new("maintenance").unwrap(),
                DisplayCondition::IsTrue,
            ));

        let ctx = DisplayContext::new()
            .with_value(ParameterKey::new("enabled").unwrap(), Value::boolean(true))
            .with_value(
                ParameterKey::new("maintenance").unwrap(),
                Value::boolean(true),
            );

        // Even though show condition is met, hide takes priority
        assert!(!display.should_display(&ctx));
    }

    #[test]
    fn test_parameter_display_default_show() {
        let display = ParameterDisplay::new();
        let ctx = DisplayContext::new();

        // No conditions = always show
        assert!(display.should_display(&ctx));
    }

    #[test]
    fn test_parameter_display_show_when_equals() {
        let display = ParameterDisplay::new()
            .show_when_equals(ParameterKey::new("mode").unwrap(), Value::text("advanced"));

        let ctx_show = DisplayContext::new()
            .with_value(ParameterKey::new("mode").unwrap(), Value::text("advanced"));

        let ctx_hide = DisplayContext::new()
            .with_value(ParameterKey::new("mode").unwrap(), Value::text("basic"));

        assert!(display.should_display(&ctx_show));
        assert!(!display.should_display(&ctx_hide));
    }

    #[test]
    fn test_parameter_display_show_when_true() {
        let display =
            ParameterDisplay::new().show_when_true(ParameterKey::new("advanced_mode").unwrap());

        let ctx_show = DisplayContext::new().with_value(
            ParameterKey::new("advanced_mode").unwrap(),
            Value::boolean(true),
        );

        let ctx_hide = DisplayContext::new().with_value(
            ParameterKey::new("advanced_mode").unwrap(),
            Value::boolean(false),
        );

        assert!(display.should_display(&ctx_show));
        assert!(!display.should_display(&ctx_hide));
    }

    #[test]
    fn test_parameter_display_hide_when_equals() {
        let display = ParameterDisplay::new().hide_when_equals(
            ParameterKey::new("status").unwrap(),
            Value::text("disabled"),
        );

        let ctx_show = DisplayContext::new()
            .with_value(ParameterKey::new("status").unwrap(), Value::text("enabled"));

        let ctx_hide = DisplayContext::new().with_value(
            ParameterKey::new("status").unwrap(),
            Value::text("disabled"),
        );

        assert!(display.should_display(&ctx_show));
        assert!(!display.should_display(&ctx_hide));
    }

    #[test]
    fn test_parameter_display_hide_when_true() {
        let display = ParameterDisplay::new().hide_when_true(ParameterKey::new("hidden").unwrap());

        let ctx_show = DisplayContext::new()
            .with_value(ParameterKey::new("hidden").unwrap(), Value::boolean(false));

        let ctx_hide = DisplayContext::new()
            .with_value(ParameterKey::new("hidden").unwrap(), Value::boolean(true));

        assert!(display.should_display(&ctx_show));
        assert!(!display.should_display(&ctx_hide));
    }

    #[test]
    fn test_parameter_display_dependencies() {
        let display = ParameterDisplay::new()
            .show_when(DisplayRule::when(
                ParameterKey::new("auth_type").unwrap(),
                DisplayCondition::Equals(Value::text("api_key")),
            ))
            .hide_when(DisplayRule::when(
                ParameterKey::new("disabled").unwrap(),
                DisplayCondition::IsTrue,
            ));

        let deps = display.dependencies();
        assert!(deps.contains(&ParameterKey::new("auth_type").unwrap()));
        assert!(deps.contains(&ParameterKey::new("disabled").unwrap()));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_parameter_display_is_empty() {
        let empty = ParameterDisplay::new();
        assert!(empty.is_empty());

        let not_empty =
            ParameterDisplay::new().show_when_true(ParameterKey::new("enabled").unwrap());
        assert!(!not_empty.is_empty());
    }

    #[test]
    fn test_parameter_display_multiple_show_conditions() {
        // Multiple show_when calls should be AND-ed
        let display = ParameterDisplay::new()
            .show_when(DisplayRule::when(
                ParameterKey::new("enabled").unwrap(),
                DisplayCondition::IsTrue,
            ))
            .show_when(DisplayRule::when(
                ParameterKey::new("level").unwrap(),
                DisplayCondition::GreaterThan(10.0),
            ));

        let ctx_both = DisplayContext::new()
            .with_value(ParameterKey::new("enabled").unwrap(), Value::boolean(true))
            .with_value(ParameterKey::new("level").unwrap(), Value::integer(15));

        let ctx_one = DisplayContext::new()
            .with_value(ParameterKey::new("enabled").unwrap(), Value::boolean(true))
            .with_value(ParameterKey::new("level").unwrap(), Value::integer(5));

        assert!(display.should_display(&ctx_both));
        assert!(!display.should_display(&ctx_one));
    }

    #[test]
    fn test_parameter_display_multiple_hide_conditions() {
        // Multiple hide_when calls should be OR-ed
        let display = ParameterDisplay::new()
            .hide_when(DisplayRule::when(
                ParameterKey::new("disabled").unwrap(),
                DisplayCondition::IsTrue,
            ))
            .hide_when(DisplayRule::when(
                ParameterKey::new("maintenance").unwrap(),
                DisplayCondition::IsTrue,
            ));

        let ctx_neither = DisplayContext::new()
            .with_value(
                ParameterKey::new("disabled").unwrap(),
                Value::boolean(false),
            )
            .with_value(
                ParameterKey::new("maintenance").unwrap(),
                Value::boolean(false),
            );

        let ctx_one = DisplayContext::new()
            .with_value(ParameterKey::new("disabled").unwrap(), Value::boolean(true))
            .with_value(
                ParameterKey::new("maintenance").unwrap(),
                Value::boolean(false),
            );

        let ctx_both = DisplayContext::new()
            .with_value(ParameterKey::new("disabled").unwrap(), Value::boolean(true))
            .with_value(
                ParameterKey::new("maintenance").unwrap(),
                Value::boolean(true),
            );

        assert!(display.should_display(&ctx_neither));
        assert!(!display.should_display(&ctx_one));
        assert!(!display.should_display(&ctx_both));
    }

    // ==========================================================================
    // Validation-based display condition tests
    // ==========================================================================

    #[test]
    fn test_condition_is_valid() {
        let rule = DisplayRule::when(
            ParameterKey::new("password").unwrap(),
            DisplayCondition::IsValid,
        );

        // Valid password
        let ctx_valid =
            DisplayContext::new().with_validation(ParameterKey::new("password").unwrap(), true);
        assert!(rule.evaluate(&ctx_valid));

        // Invalid password
        let ctx_invalid =
            DisplayContext::new().with_validation(ParameterKey::new("password").unwrap(), false);
        assert!(!rule.evaluate(&ctx_invalid));

        // No validation state
        let ctx_none = DisplayContext::new();
        assert!(!rule.evaluate(&ctx_none));
    }

    #[test]
    fn test_condition_is_invalid() {
        let rule = DisplayRule::when(
            ParameterKey::new("email").unwrap(),
            DisplayCondition::IsInvalid,
        );

        // Invalid email
        let ctx_invalid =
            DisplayContext::new().with_validation(ParameterKey::new("email").unwrap(), false);
        assert!(rule.evaluate(&ctx_invalid));

        // Valid email
        let ctx_valid =
            DisplayContext::new().with_validation(ParameterKey::new("email").unwrap(), true);
        assert!(!rule.evaluate(&ctx_valid));

        // No validation state
        let ctx_none = DisplayContext::new();
        assert!(!rule.evaluate(&ctx_none));
    }

    #[test]
    fn test_condition_is_valid_evaluate_on_value_returns_false() {
        // IsValid/IsInvalid conditions cannot be evaluated against values directly
        let condition = DisplayCondition::IsValid;
        assert!(!condition.evaluate(&Value::boolean(true)));
        assert!(!condition.evaluate(&Value::text("valid")));

        let condition = DisplayCondition::IsInvalid;
        assert!(!condition.evaluate(&Value::boolean(false)));
        assert!(!condition.evaluate(&Value::text("invalid")));
    }

    #[test]
    fn test_condition_requires_validation_state() {
        assert!(DisplayCondition::IsValid.requires_validation_state());
        assert!(DisplayCondition::IsInvalid.requires_validation_state());
        assert!(!DisplayCondition::IsTrue.requires_validation_state());
        assert!(!DisplayCondition::Equals(Value::integer(1)).requires_validation_state());
    }

    #[test]
    fn test_display_context_validation_methods() {
        let mut ctx = DisplayContext::new();

        // Initially no validation state
        assert_eq!(ctx.get_validation("field"), None);
        assert!(!ctx.is_valid("field"));
        assert!(!ctx.is_invalid("field"));

        // Mark as valid
        ctx.mark_valid(ParameterKey::new("field").unwrap());
        assert_eq!(ctx.get_validation("field"), Some(true));
        assert!(ctx.is_valid("field"));
        assert!(!ctx.is_invalid("field"));

        // Mark as invalid
        ctx.mark_invalid(ParameterKey::new("field").unwrap());
        assert_eq!(ctx.get_validation("field"), Some(false));
        assert!(!ctx.is_valid("field"));
        assert!(ctx.is_invalid("field"));

        // Clear validation
        ctx.clear_validation("field");
        assert_eq!(ctx.get_validation("field"), None);
        assert!(!ctx.is_valid("field"));
        assert!(!ctx.is_invalid("field"));
    }

    #[test]
    fn test_display_context_with_validation_builder() {
        let ctx = DisplayContext::new()
            .with_value(
                ParameterKey::new("password").unwrap(),
                Value::text("secret"),
            )
            .with_validation(ParameterKey::new("password").unwrap(), true)
            .with_validation(ParameterKey::new("email").unwrap(), false);

        assert!(ctx.is_valid("password"));
        assert!(ctx.is_invalid("email"));
        assert_eq!(ctx.get("password"), Some(&Value::text("secret")));
    }

    #[test]
    fn test_parameter_display_show_when_valid() {
        // Show "Confirm Password" only when "Password" is valid
        let display =
            ParameterDisplay::new().show_when_valid(ParameterKey::new("password").unwrap());

        let ctx_valid =
            DisplayContext::new().with_validation(ParameterKey::new("password").unwrap(), true);
        let ctx_invalid =
            DisplayContext::new().with_validation(ParameterKey::new("password").unwrap(), false);
        let ctx_none = DisplayContext::new();

        assert!(display.should_display(&ctx_valid));
        assert!(!display.should_display(&ctx_invalid));
        assert!(!display.should_display(&ctx_none));
    }

    #[test]
    fn test_parameter_display_show_when_invalid() {
        // Show error hint when email is invalid
        let display =
            ParameterDisplay::new().show_when_invalid(ParameterKey::new("email").unwrap());

        let ctx_invalid =
            DisplayContext::new().with_validation(ParameterKey::new("email").unwrap(), false);
        let ctx_valid =
            DisplayContext::new().with_validation(ParameterKey::new("email").unwrap(), true);

        assert!(display.should_display(&ctx_invalid));
        assert!(!display.should_display(&ctx_valid));
    }

    #[test]
    fn test_parameter_display_hide_when_invalid() {
        // Hide submit button when form has invalid fields
        let display =
            ParameterDisplay::new().hide_when_invalid(ParameterKey::new("required_field").unwrap());

        let ctx_valid = DisplayContext::new()
            .with_validation(ParameterKey::new("required_field").unwrap(), true);
        let ctx_invalid = DisplayContext::new()
            .with_validation(ParameterKey::new("required_field").unwrap(), false);

        assert!(display.should_display(&ctx_valid));
        assert!(!display.should_display(&ctx_invalid));
    }

    #[test]
    fn test_combined_value_and_validation_conditions() {
        // Show "Advanced Settings" when mode is "advanced" AND basic config is valid
        let display = ParameterDisplay::new().show_when(DisplayRuleSet::all([
            DisplayRule::when(
                ParameterKey::new("mode").unwrap(),
                DisplayCondition::Equals(Value::text("advanced")),
            ),
            DisplayRule::when(
                ParameterKey::new("basic_config").unwrap(),
                DisplayCondition::IsValid,
            ),
        ]));

        // Both conditions met
        let ctx_both = DisplayContext::new()
            .with_value(ParameterKey::new("mode").unwrap(), Value::text("advanced"))
            .with_validation(ParameterKey::new("basic_config").unwrap(), true);
        assert!(display.should_display(&ctx_both));

        // Value matches but validation fails
        let ctx_value_only = DisplayContext::new()
            .with_value(ParameterKey::new("mode").unwrap(), Value::text("advanced"))
            .with_validation(ParameterKey::new("basic_config").unwrap(), false);
        assert!(!display.should_display(&ctx_value_only));

        // Validation passes but value doesn't match
        let ctx_valid_only = DisplayContext::new()
            .with_value(ParameterKey::new("mode").unwrap(), Value::text("simple"))
            .with_validation(ParameterKey::new("basic_config").unwrap(), true);
        assert!(!display.should_display(&ctx_valid_only));
    }

    #[test]
    fn test_validation_state_serialization() {
        let condition_valid = DisplayCondition::IsValid;
        let json = serde_json::to_string(&condition_valid).expect("Failed to serialize");
        let deserialized: DisplayCondition =
            serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(condition_valid, deserialized);

        let condition_invalid = DisplayCondition::IsInvalid;
        let json = serde_json::to_string(&condition_invalid).expect("Failed to serialize");
        let deserialized: DisplayCondition =
            serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(condition_invalid, deserialized);
    }

    #[test]
    fn test_validation_conditions_in_complex_ruleset() {
        // Show when: (password valid AND confirm_password valid) OR admin_override is true
        let ruleset = DisplayRuleSet::any([
            DisplayRuleSet::all([
                DisplayRule::when(
                    ParameterKey::new("password").unwrap(),
                    DisplayCondition::IsValid,
                ),
                DisplayRule::when(
                    ParameterKey::new("confirm_password").unwrap(),
                    DisplayCondition::IsValid,
                ),
            ]),
            DisplayRuleSet::single(DisplayRule::when(
                ParameterKey::new("admin_override").unwrap(),
                DisplayCondition::IsTrue,
            )),
        ]);

        // Both passwords valid
        let ctx_valid = DisplayContext::new()
            .with_validation(ParameterKey::new("password").unwrap(), true)
            .with_validation(ParameterKey::new("confirm_password").unwrap(), true);
        assert!(ruleset.evaluate(&ctx_valid));

        // Admin override
        let ctx_admin = DisplayContext::new().with_value(
            ParameterKey::new("admin_override").unwrap(),
            Value::boolean(true),
        );
        assert!(ruleset.evaluate(&ctx_admin));

        // Neither condition met
        let ctx_none = DisplayContext::new()
            .with_validation(ParameterKey::new("password").unwrap(), true)
            .with_validation(ParameterKey::new("confirm_password").unwrap(), false);
        assert!(!ruleset.evaluate(&ctx_none));
    }
}
