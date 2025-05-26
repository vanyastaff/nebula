use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::AsRefStr;
use thiserror::Error;

/// Error types for parameter validation failures
#[derive(Debug, Error, PartialEq, Clone)]
pub enum ParameterCheckError {
    #[error(
        "Type mismatch for condition '{condition_variant}': expected {expected_type}, got \
         {actual_type}"
    )]
    TypeMismatch {
        condition_variant: String,
        expected_type: String,
        actual_type: String,
    },

    #[error("Regex compilation error for pattern '{pattern}': {error}")]
    RegexCompileError { pattern: String, error: String },

    #[error("Regex comparison failed: expected '{expected}', got '{actual}'")]
    RegexComparisonFailed { expected: String, actual: String },

    #[error("Numeric comparison failed ({operator}): expected {expected}, got {actual}")]
    NumericComparisonFailed {
        operator: String,
        expected: f64,
        actual: f64,
    },

    #[error("String comparison failed ({operator}): expected '{expected}', got '{actual}'")]
    StringComparisonFailed {
        operator: String,
        expected: String,
        actual: String,
    },

    #[error("String is too short: expected at least {min} characters, got {actual}")]
    StringLengthTooShort { min: usize, actual: usize },

    #[error("String is too long: expected at most {max} characters, got {actual}")]
    StringLengthTooLong { max: usize, actual: usize },

    #[error("Between comparison failed: expected value between {from} and {to}, but got {actual}")]
    BetweenFailed { from: f64, to: f64, actual: f64 },

    #[error(
        "Empty check failed (expected empty = {expected_empty}): got {actual_value_description}"
    )]
    EmptyCheckFailed {
        expected_empty: bool,
        actual_value_description: String,
    },

    #[error("Logical AND failed with {count} errors: {errors:?}")]
    LogicalAndFailed {
        count: usize,
        errors: Vec<ParameterCheckError>,
    },

    #[error("Logical OR failed; all {count} conditions failed: {errors:?}")]
    LogicalOrFailed {
        count: usize,
        errors: Vec<ParameterCheckError>,
    },

    #[error("Logical NOT failed: inner condition passed unexpectedly")]
    LogicalNotFailed,

    #[error(
        "Value comparison failed ({operator}): expected {expected_display}, got {actual_display}"
    )]
    ValueComparisonFailed {
        operator: String,
        expected: Value,
        actual: Value,
        expected_display: String,
        actual_display: String,
    },

    #[error("An unexpected error occurred: {0}")]
    Other(String),
}

/// Conditions that can be applied to parameters for validation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ParameterCondition {
    /// Equal to the specified value
    Eq(Value),

    /// Not equal to the specified value
    NotEq(Value),

    /// String length greater than the specified value
    StringMinLength(usize),

    /// String length less than the specified value
    StringMaxLength(usize),

    /// Greater than or equal to the specified numeric value
    Gte(Value),

    /// Greater than the specified numeric value
    Gt(Value),

    /// Less than the specified numeric value
    Lt(Value),

    /// Less than or equal to the specified numeric value
    Lte(Value),

    /// Between the specified numeric values (inclusive)
    Between { from: Value, to: Value },

    /// String starts with the specified prefix
    StartsWith(Value),

    /// String ends with the specified suffix
    EndsWith(Value),

    /// String contains the specified substring
    Contains(Value),

    /// String matches the specified regex pattern
    Regex(Value),

    /// Value is empty (empty string, empty array, empty object, or null)
    IsEmpty,

    /// Value is not empty (non-empty string, non-empty array, non-empty object,
    /// and not null)
    IsNotEmpty,

    /// All conditions must be satisfied (logical AND)
    And(Vec<ParameterCondition>),

    /// At least one condition must be satisfied (logical OR)
    Or(Vec<ParameterCondition>),

    /// The inner condition must not be satisfied (logical NOT)
    Not(Box<ParameterCondition>),
}

impl ParameterCondition {
    /// Check if a value satisfies this condition
    pub fn check(&self, value: &Value) -> Result<(), ParameterCheckError> {
        match self {
            ParameterCondition::Eq(exp) => {
                if value == exp {
                    Ok(())
                } else {
                    Err(ParameterCheckError::ValueComparisonFailed {
                        operator: self.as_ref().to_string(),
                        expected: exp.clone(),
                        actual: value.clone(),
                        expected_display: Self::format_value(exp),
                        actual_display: Self::format_value(value),
                    })
                }
            }
            ParameterCondition::NotEq(exp) => {
                if value != exp {
                    Ok(())
                } else {
                    Err(ParameterCheckError::ValueComparisonFailed {
                        operator: self.as_ref().to_string(),
                        expected: exp.clone(),
                        actual: value.clone(),
                        expected_display: format!("not {}", Self::format_value(exp)),
                        actual_display: Self::format_value(value),
                    })
                }
            }
            ParameterCondition::StringMinLength(min_length) => {
                if let Some(s) = value.as_str() {
                    if s.len() >= *min_length {
                        Ok(())
                    } else {
                        Err(ParameterCheckError::StringLengthTooShort {
                            min: *min_length,
                            actual: s.len(),
                        })
                    }
                } else {
                    Err(ParameterCheckError::TypeMismatch {
                        condition_variant: "string_min_length".into(),
                        expected_type: "string".into(),
                        actual_type: Self::value_type_name(value),
                    })
                }
            }

            ParameterCondition::StringMaxLength(max_length) => {
                if let Some(s) = value.as_str() {
                    if s.len() <= *max_length {
                        Ok(())
                    } else {
                        Err(ParameterCheckError::StringLengthTooLong {
                            max: *max_length,
                            actual: s.len(),
                        })
                    }
                } else {
                    Err(ParameterCheckError::TypeMismatch {
                        condition_variant: "string_max_length".into(),
                        expected_type: "string".into(),
                        actual_type: Self::value_type_name(value),
                    })
                }
            }
            ParameterCondition::Gte(exp) => self.check_numeric(value, exp, |a, b| a >= b, ">="),
            ParameterCondition::Gt(exp) => self.check_numeric(value, exp, |a, b| a > b, ">"),
            ParameterCondition::Lt(exp) => self.check_numeric(value, exp, |a, b| a < b, "<"),
            ParameterCondition::Lte(exp) => self.check_numeric(value, exp, |a, b| a <= b, "<="),
            ParameterCondition::Between { from, to } => {
                let actual = Self::extract_f64(value, self.as_ref())?;
                let start = Self::extract_f64(from, self.as_ref())?;
                let end = Self::extract_f64(to, self.as_ref())?;
                if (start..=end).contains(&actual) {
                    Ok(())
                } else {
                    Err(ParameterCheckError::BetweenFailed {
                        from: start,
                        to: end,
                        actual,
                    })
                }
            }
            ParameterCondition::StartsWith(pref) => {
                self.check_string(value, pref, |v, p| v.starts_with(p), "starts_with")
            }
            ParameterCondition::EndsWith(suff) => {
                self.check_string(value, suff, |v, s| v.ends_with(s), "ends_with")
            }
            ParameterCondition::Contains(sub) => {
                self.check_string(value, sub, |v, s| v.contains(s), "contains")
            }
            ParameterCondition::Regex(pattern) => {
                let text = Self::extract_str(value, self.as_ref())?;
                let pat = Self::extract_str(pattern, self.as_ref())?;
                let re = Regex::new(pat).map_err(|e| ParameterCheckError::RegexCompileError {
                    pattern: pat.into(),
                    error: e.to_string(),
                })?;
                if re.is_match(text) {
                    Ok(())
                } else {
                    Err(ParameterCheckError::RegexComparisonFailed {
                        expected: pat.to_string(),
                        actual: text.to_string(),
                    })
                }
            }
            ParameterCondition::IsEmpty => match value {
                Value::String(s) if s.is_empty() => Ok(()),
                Value::Array(a) if a.is_empty() => Ok(()),
                Value::Object(o) if o.is_empty() => Ok(()),
                Value::Null => Ok(()),
                _ => Err(ParameterCheckError::EmptyCheckFailed {
                    expected_empty: true,
                    actual_value_description: format!("not empty: {}", Self::format_value(value)),
                }),
            },
            ParameterCondition::IsNotEmpty => match value {
                Value::String(s) if !s.is_empty() => Ok(()),
                Value::Array(a) if !a.is_empty() => Ok(()),
                Value::Object(o) if !o.is_empty() => Ok(()),
                Value::Null => Err(ParameterCheckError::EmptyCheckFailed {
                    expected_empty: false,
                    actual_value_description: "null".into(),
                }),
                _ => Err(ParameterCheckError::EmptyCheckFailed {
                    expected_empty: false,
                    actual_value_description: format!(
                        "empty or not-applicable: {}",
                        Self::format_value(value)
                    ),
                }),
            },
            ParameterCondition::And(conds) => {
                let mut errs = Vec::new();
                for c in conds {
                    if let Err(e) = c.check(value) {
                        errs.push(e);
                    }
                }
                if errs.is_empty() {
                    Ok(())
                } else {
                    Err(ParameterCheckError::LogicalAndFailed {
                        count: errs.len(),
                        errors: errs,
                    })
                }
            }
            ParameterCondition::Or(conds) => {
                let mut errs = Vec::new();
                for c in conds {
                    match c.check(value) {
                        Ok(_) => return Ok(()),
                        Err(e) => errs.push(e),
                    }
                }
                Err(ParameterCheckError::LogicalOrFailed {
                    count: errs.len(),
                    errors: errs,
                })
            }
            ParameterCondition::Not(c) => match c.check(value) {
                Ok(_) => Err(ParameterCheckError::LogicalNotFailed),
                Err(_) => Ok(()),
            },
        }
    }

    /// Helper function to perform numeric comparisons
    fn check_numeric(
        &self,
        value: &Value,
        exp: &Value,
        cmp: impl Fn(f64, f64) -> bool,
        op: &'static str,
    ) -> Result<(), ParameterCheckError> {
        let a = Self::extract_f64(value, self.as_ref())?;
        let b = Self::extract_f64(exp, self.as_ref())?;
        if cmp(a, b) {
            Ok(())
        } else {
            Err(ParameterCheckError::NumericComparisonFailed {
                operator: op.into(),
                expected: b,
                actual: a,
            })
        }
    }

    /// Helper function to perform string comparisons
    fn check_string(
        &self,
        value: &Value,
        exp: &Value,
        cmp: impl Fn(&str, &str) -> bool,
        op: &'static str,
    ) -> Result<(), ParameterCheckError> {
        let a = Self::extract_str(value, self.as_ref())?;
        let b = Self::extract_str(exp, self.as_ref())?;
        if cmp(a, b) {
            Ok(())
        } else {
            Err(ParameterCheckError::StringComparisonFailed {
                operator: op.into(),
                expected: b.to_string(),
                actual: a.to_string(),
            })
        }
    }

    /// Helper function to extract a float from a Value
    fn extract_f64(v: &Value, var: &str) -> Result<f64, ParameterCheckError> {
        v.as_f64().ok_or(ParameterCheckError::TypeMismatch {
            condition_variant: var.into(),
            expected_type: "number".into(),
            actual_type: Self::value_type_name(v),
        })
    }

    /// Helper function to extract a string from a Value
    fn extract_str<'a>(v: &'a Value, var: &str) -> Result<&'a str, ParameterCheckError> {
        v.as_str().ok_or(ParameterCheckError::TypeMismatch {
            condition_variant: var.into(),
            expected_type: "string".into(),
            actual_type: Self::value_type_name(v),
        })
    }

    /// Get a human-readable type name for a Value
    fn value_type_name(v: &Value) -> String {
        match v {
            Value::Null => "null".into(),
            Value::Bool(_) => "boolean".into(),
            Value::Number(_) => "number".into(),
            Value::String(_) => "string".into(),
            Value::Array(_) => "array".into(),
            Value::Object(_) => "object".into(),
        }
    }

    /// Format a JSON value for error displays
    fn format_value(v: &Value) -> String {
        match v {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("\"{}\"", s),
            Value::Array(_) | Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
        }
    }

    // Helper constructors for common conditions

    /// Create an equality condition
    pub fn equals<T: Into<Value>>(value: T) -> Self {
        ParameterCondition::Eq(value.into())
    }

    /// Create a non-equality condition
    pub fn not_equals<T: Into<Value>>(value: T) -> Self {
        ParameterCondition::NotEq(value.into())
    }

    /// Create a greater-than condition
    pub fn greater_than<T: Into<Value>>(value: T) -> Self {
        ParameterCondition::Gt(value.into())
    }

    /// Create a less-than condition
    pub fn less_than<T: Into<Value>>(value: T) -> Self {
        ParameterCondition::Lt(value.into())
    }

    /// Create a between condition
    pub fn between<T: Into<Value>, U: Into<Value>>(from: T, to: U) -> Self {
        ParameterCondition::Between {
            from: from.into(),
            to: to.into(),
        }
    }

    /// Create a regex pattern match condition
    pub fn regex_pattern<T: Into<String>>(pattern: T) -> Self {
        ParameterCondition::Regex(Value::String(pattern.into()))
    }

    /// Create a string contains condition
    pub fn contains<T: Into<String>>(substring: T) -> Self {
        ParameterCondition::Contains(Value::String(substring.into()))
    }

    /// Combine multiple conditions with AND logic
    pub fn all(conditions: Vec<ParameterCondition>) -> Self {
        ParameterCondition::And(conditions)
    }

    /// Combine multiple conditions with OR logic
    pub fn any(conditions: Vec<ParameterCondition>) -> Self {
        ParameterCondition::Or(conditions)
    }

    /// Negate a condition
    pub fn not(condition: ParameterCondition) -> Self {
        ParameterCondition::Not(Box::new(condition))
    }
}

// Unit tests for the ParameterCondition implementation
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_eq_condition() {
        let cond = ParameterCondition::Eq(json!(42));

        // Equal case
        assert!(cond.check(&json!(42)).is_ok());

        // Not equal case
        assert!(cond.check(&json!(43)).is_err());
        assert!(cond.check(&json!("42")).is_err());
    }

    #[test]
    fn test_between_condition() {
        let cond = ParameterCondition::Between {
            from: json!(10),
            to: json!(20),
        };

        // Within range
        assert!(cond.check(&json!(10)).is_ok());
        assert!(cond.check(&json!(15)).is_ok());
        assert!(cond.check(&json!(20)).is_ok());

        // Outside range
        assert!(cond.check(&json!(9)).is_err());
        assert!(cond.check(&json!(21)).is_err());

        // Type mismatch
        assert!(cond.check(&json!("15")).is_err());
    }

    #[test]
    fn test_string_conditions() {
        let starts_with = ParameterCondition::StartsWith(json!("hello"));
        let ends_with = ParameterCondition::EndsWith(json!("world"));
        let contains = ParameterCondition::Contains(json!("lo wo"));

        // String that matches all conditions
        let value = json!("hello world");
        assert!(starts_with.check(&value).is_ok());
        assert!(ends_with.check(&value).is_ok());
        assert!(contains.check(&value).is_ok());

        // Strings that don't match
        assert!(starts_with.check(&json!("hi world")).is_err());
        assert!(ends_with.check(&json!("hello there")).is_err());
        assert!(contains.check(&json!("helloworld")).is_err());

        // Type mismatch
        assert!(starts_with.check(&json!(42)).is_err());
    }

    #[test]
    fn test_regex_condition() {
        let cond = ParameterCondition::Regex(json!("^\\d{3}-\\d{2}-\\d{4}$"));

        // Matching values
        assert!(cond.check(&json!("123-45-6789")).is_ok());

        // Non-matching values
        assert!(cond.check(&json!("123-456-789")).is_err());
        assert!(cond.check(&json!("abc-12-3456")).is_err());

        // Invalid regex
        let invalid_regex = ParameterCondition::Regex(json!("(unclosed"));
        assert!(matches!(
            invalid_regex.check(&json!("test")),
            Err(ParameterCheckError::RegexCompileError { .. })
        ));
    }

    #[test]
    fn test_empty_conditions() {
        let is_empty = ParameterCondition::IsEmpty;
        let is_not_empty = ParameterCondition::IsNotEmpty;

        // Empty values
        assert!(is_empty.check(&json!("")).is_ok());
        assert!(is_empty.check(&json!([])).is_ok());
        assert!(is_empty.check(&json!({})).is_ok());
        assert!(is_empty.check(&json!(null)).is_ok());

        // Non-empty values
        assert!(is_not_empty.check(&json!("hello")).is_ok());
        assert!(is_not_empty.check(&json!([1, 2, 3])).is_ok());
        assert!(is_not_empty.check(&json!({"key": "value"})).is_ok());

        // Failures
        assert!(is_empty.check(&json!("hello")).is_err());
        assert!(is_not_empty.check(&json!("")).is_err());
        assert!(is_not_empty.check(&json!(null)).is_err());
    }

    #[test]
    fn test_logical_operators() {
        // AND condition
        let and_cond = ParameterCondition::And(vec![
            ParameterCondition::Gt(json!(10)),
            ParameterCondition::Lt(json!(20)),
        ]);

        assert!(and_cond.check(&json!(15)).is_ok());
        assert!(and_cond.check(&json!(5)).is_err());
        assert!(and_cond.check(&json!(25)).is_err());

        // OR condition
        let or_cond = ParameterCondition::Or(vec![
            ParameterCondition::Lt(json!(10)),
            ParameterCondition::Gt(json!(20)),
        ]);

        assert!(or_cond.check(&json!(5)).is_ok());
        assert!(or_cond.check(&json!(25)).is_ok());
        assert!(or_cond.check(&json!(15)).is_err());

        // NOT condition
        let not_cond = ParameterCondition::Not(Box::new(ParameterCondition::Eq(json!("hello"))));

        assert!(not_cond.check(&json!("world")).is_ok());
        assert!(not_cond.check(&json!("hello")).is_err());
    }

    #[test]
    fn test_helper_constructors() {
        // Test the helper constructors
        let equals = ParameterCondition::equals(42);
        assert_eq!(equals, ParameterCondition::Eq(json!(42)));

        let between = ParameterCondition::between(10, 20);
        assert_eq!(
            between,
            ParameterCondition::Between {
                from: json!(10),
                to: json!(20),
            }
        );

        let all = ParameterCondition::all(vec![
            ParameterCondition::equals(42),
            ParameterCondition::greater_than(10),
        ]);
        assert!(matches!(all, ParameterCondition::And(_)));

        let not = ParameterCondition::not(ParameterCondition::equals(42));
        assert!(matches!(not, ParameterCondition::Not(_)));
    }
}
