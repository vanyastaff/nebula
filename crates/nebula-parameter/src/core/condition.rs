use crate::core::ParameterValue;
use nebula_value::Value;
use serde::{Deserialize, Serialize};

/// Condition for parameter validation and display with enhanced functionality
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParameterCondition {
    // Comparison
    Eq(ParameterValue),
    NotEq(ParameterValue),
    Gt(ParameterValue),
    Gte(ParameterValue),
    Lt(ParameterValue),
    Lte(ParameterValue),
    Between {
        from: ParameterValue,
        to: ParameterValue,
    },

    // String operations
    StartsWith(ParameterValue),
    EndsWith(ParameterValue),
    Contains(ParameterValue),
    Regex(ParameterValue),
    StringMinLength(usize),
    StringMaxLength(usize),

    // Set operations
    In(Vec<ParameterValue>),
    NotIn(Vec<ParameterValue>),

    // Existence
    IsEmpty,
    IsNotEmpty,

    // Logical operators
    And(Vec<ParameterCondition>),
    Or(Vec<ParameterCondition>),
    Not(Box<ParameterCondition>),
}

impl ParameterCondition {
    /// Evaluate the condition against a value
    pub fn evaluate(&self, value: &ParameterValue) -> bool {
        match self {
            ParameterCondition::Eq(expected) => value == expected,
            ParameterCondition::NotEq(expected) => value != expected,
            ParameterCondition::Gt(expected) => {
                Self::compare_numbers(value, expected, |a, b| a > b)
            }
            ParameterCondition::Gte(expected) => {
                Self::compare_numbers(value, expected, |a, b| a >= b)
            }
            ParameterCondition::Lt(expected) => {
                Self::compare_numbers(value, expected, |a, b| a < b)
            }
            ParameterCondition::Lte(expected) => {
                Self::compare_numbers(value, expected, |a, b| a <= b)
            }
            ParameterCondition::Between { from, to } => {
                Self::compare_numbers(value, from, |a, b| a >= b)
                    && Self::compare_numbers(value, to, |a, b| a <= b)
            }
            ParameterCondition::StartsWith(prefix) => {
                Self::compare_strings(value, prefix, |s, p| s.starts_with(p))
            }
            ParameterCondition::EndsWith(suffix) => {
                Self::compare_strings(value, suffix, |s, suf| s.ends_with(suf))
            }
            ParameterCondition::Contains(substring) => {
                Self::compare_strings(value, substring, |s, sub| s.contains(sub))
            }
            ParameterCondition::Regex(pattern) => Self::evaluate_regex(value, pattern),
            ParameterCondition::StringMinLength(min_len) => {
                Self::check_string_length(value, |len| len >= *min_len)
            }
            ParameterCondition::StringMaxLength(max_len) => {
                Self::check_string_length(value, |len| len <= *max_len)
            }
            ParameterCondition::In(values) => values.contains(value),
            ParameterCondition::NotIn(values) => !values.contains(value),
            ParameterCondition::IsEmpty => {
                // Check if MaybeExpression is empty
                match value {
                    ParameterValue::Value(v) => match v {
                        nebula_value::Value::Null => true,
                        nebula_value::Value::Text(s) => s.is_empty(),
                        nebula_value::Value::Array(a) => a.is_empty(),
                        nebula_value::Value::Object(o) => o.is_empty(),
                        _ => false,
                    },
                    ParameterValue::Expression(expr) => expr.is_empty(),
                }
            }
            ParameterCondition::IsNotEmpty => {
                // Check if MaybeExpression is not empty
                match value {
                    ParameterValue::Value(v) => match v {
                        nebula_value::Value::Null => false,
                        nebula_value::Value::Text(s) => !s.is_empty(),
                        nebula_value::Value::Array(a) => !a.is_empty(),
                        nebula_value::Value::Object(o) => !o.is_empty(),
                        _ => true,
                    },
                    ParameterValue::Expression(expr) => !expr.is_empty(),
                }
            }
            ParameterCondition::And(conditions) => conditions.iter().all(|c| c.evaluate(value)),
            ParameterCondition::Or(conditions) => conditions.iter().any(|c| c.evaluate(value)),
            ParameterCondition::Not(condition) => !condition.evaluate(value),
        }
    }

    // Helper constructor methods

    /// Create an equality condition
    pub fn equals<T: Into<ParameterValue>>(value: T) -> Self {
        ParameterCondition::Eq(value.into())
    }

    /// Create a not equals condition
    pub fn not_equals<T: Into<ParameterValue>>(value: T) -> Self {
        ParameterCondition::NotEq(value.into())
    }

    /// Create a greater than condition
    pub fn greater_than<T: Into<ParameterValue>>(value: T) -> Self {
        ParameterCondition::Gt(value.into())
    }

    /// Create a less than condition
    pub fn less_than<T: Into<ParameterValue>>(value: T) -> Self {
        ParameterCondition::Lt(value.into())
    }

    /// Create a between condition
    pub fn between<T: Into<ParameterValue>, U: Into<ParameterValue>>(from: T, to: U) -> Self {
        ParameterCondition::Between {
            from: from.into(),
            to: to.into(),
        }
    }

    /// Create a regex pattern condition
    pub fn regex_pattern<T: Into<String>>(pattern: T) -> Self {
        ParameterCondition::Regex(ParameterValue::from(Value::text(pattern.into())))
    }

    /// Create a contains condition
    pub fn contains<T: Into<String>>(substring: T) -> Self {
        ParameterCondition::Contains(ParameterValue::from(Value::text(substring.into())))
    }

    /// Create an AND condition
    pub fn all(conditions: Vec<ParameterCondition>) -> Self {
        ParameterCondition::And(conditions)
    }

    /// Create an OR condition
    pub fn any(conditions: Vec<ParameterCondition>) -> Self {
        ParameterCondition::Or(conditions)
    }

    /// Create a NOT condition
    pub fn not(condition: ParameterCondition) -> Self {
        ParameterCondition::Not(Box::new(condition))
    }

    // Helper functions for evaluation

    /// Helper function to compare two numeric values
    #[inline]
    fn compare_numbers<F>(value: &ParameterValue, expected: &ParameterValue, op: F) -> bool
    where
        F: Fn(f64, f64) -> bool,
    {
        match (value.as_value(), expected.as_value()) {
            (Some(Value::Integer(a)), Some(Value::Integer(b))) => {
                op(a.value() as f64, b.value() as f64)
            }
            (Some(Value::Float(a)), Some(Value::Float(b))) => op(a.value(), b.value()),
            (Some(Value::Integer(a)), Some(Value::Float(b))) => op(a.value() as f64, b.value()),
            (Some(Value::Float(a)), Some(Value::Integer(b))) => op(a.value(), b.value() as f64),
            _ => false,
        }
    }

    /// Helper function to compare two string values
    #[inline]
    fn compare_strings<F>(value: &ParameterValue, expected: &ParameterValue, op: F) -> bool
    where
        F: Fn(&str, &str) -> bool,
    {
        match (value.as_value(), expected.as_value()) {
            (Some(Value::Text(a)), Some(Value::Text(b))) => op(a, b),
            _ => false,
        }
    }

    /// Helper function to check string length
    #[inline]
    fn check_string_length<F>(value: &ParameterValue, op: F) -> bool
    where
        F: Fn(usize) -> bool,
    {
        match value.as_value() {
            Some(Value::Text(s)) => op(s.len()),
            _ => false,
        }
    }

    /// Evaluate regex pattern
    fn evaluate_regex(value: &ParameterValue, pattern: &ParameterValue) -> bool {
        match (value.as_value(), pattern.as_value()) {
            (Some(Value::Text(s)), Some(Value::Text(p))) => {
                // Enhanced regex patterns for common validation cases
                match p.as_str() {
                    // Email pattern
                    r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$" => Self::validate_email(s),
                    // URL patterns
                    r"^https?://[^\s/$.?#].[^\s]*$" => {
                        s.starts_with("http://") || s.starts_with("https://")
                    }
                    r"^https://[^\s/$.?#].[^\s]*$" => s.starts_with("https://"),
                    // Phone number pattern
                    r"^\+?[\d\s\-\(\)]+$" => {
                        s.chars().all(|c| c.is_digit(10) || "+- ()".contains(c))
                    }
                    // Credit card pattern
                    r"^\d{4}[\s\-]?\d{4}[\s\-]?\d{4}[\s\-]?\d{4}$" => Self::validate_credit_card(s),
                    // Password strength patterns
                    r"[A-Z]" => s.chars().any(|c| c.is_uppercase()),
                    r"[a-z]" => s.chars().any(|c| c.is_lowercase()),
                    r"\d" => s.chars().any(|c| c.is_digit(10)),
                    r"[!@#$%^&*]" => s.chars().any(|c| "!@#$%^&*".contains(c)),
                    // Simple patterns that can use contains
                    _ if p.len() <= 10 && !p.contains("^") && !p.contains("$") => s.contains(p),
                    // For complex patterns, fall back to basic contains check
                    _ => s.contains(p.trim_start_matches('^').trim_end_matches('$')),
                }
            }
            _ => false,
        }
    }

    /// Optimized email validation
    #[inline]
    fn validate_email(s: &str) -> bool {
        // Fast path checks first
        if s.len() < 5 || !s.contains('@') || !s.contains('.') {
            return false;
        }

        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() != 2 {
            return false;
        }

        let (local, domain) = (parts[0], parts[1]);
        if local.is_empty() || domain.is_empty() || !domain.contains('.') {
            return false;
        }

        // Check for valid characters
        s.chars()
            .all(|c| c.is_alphanumeric() || "@._%+-".contains(c))
    }

    /// Optimized credit card validation
    #[inline]
    fn validate_credit_card(s: &str) -> bool {
        let cleaned: String = s.chars().filter(|c| c.is_digit(10)).collect();
        cleaned.len() == 16
    }
}
