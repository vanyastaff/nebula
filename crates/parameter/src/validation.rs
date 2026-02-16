use serde::{Deserialize, Serialize};

/// A declarative validation rule that can be attached to a parameter.
///
/// These are pure data descriptions of constraints. Actual validation
/// logic lives in the engine or in `nebula-validator` — this crate only
/// defines the rule shapes so they can be serialized into parameter schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum ValidationRule {
    /// String must be at least `length` characters.
    MinLength {
        length: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// String must be at most `length` characters.
    MaxLength {
        length: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// String must match the given regex pattern.
    Pattern {
        pattern: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Numeric value must be >= `value`.
    Min {
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Numeric value must be <= `value`.
    Max {
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Value must be one of the given allowed values.
    OneOf {
        values: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Value must satisfy a custom expression (evaluated by the engine).
    Custom {
        expression: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Collection must contain at least `count` items.
    MinItems {
        count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Collection must contain at most `count` items.
    MaxItems {
        count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

impl ValidationRule {
    /// Require a minimum string length.
    #[must_use]
    pub fn min_length(length: usize) -> Self {
        Self::MinLength {
            length,
            message: None,
        }
    }

    /// Require a maximum string length.
    #[must_use]
    pub fn max_length(length: usize) -> Self {
        Self::MaxLength {
            length,
            message: None,
        }
    }

    /// Require a string to match a regex pattern.
    #[must_use]
    pub fn pattern(pattern: impl Into<String>) -> Self {
        Self::Pattern {
            pattern: pattern.into(),
            message: None,
        }
    }

    /// Require a numeric minimum (inclusive).
    #[must_use]
    pub fn min(value: f64) -> Self {
        Self::Min {
            value,
            message: None,
        }
    }

    /// Require a numeric maximum (inclusive).
    #[must_use]
    pub fn max(value: f64) -> Self {
        Self::Max {
            value,
            message: None,
        }
    }

    /// Require a numeric value within an inclusive range.
    ///
    /// Returns a pair of `[Min, Max]` rules.
    #[must_use]
    pub fn range(min: f64, max: f64) -> Vec<Self> {
        vec![Self::min(min), Self::max(max)]
    }

    /// Require a minimum number of items in a collection.
    #[must_use]
    pub fn min_items(count: usize) -> Self {
        Self::MinItems {
            count,
            message: None,
        }
    }

    /// Require a maximum number of items in a collection.
    #[must_use]
    pub fn max_items(count: usize) -> Self {
        Self::MaxItems {
            count,
            message: None,
        }
    }

    /// Set a custom error message on this rule (builder-style).
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        match &mut self {
            Self::MinLength { message: m, .. }
            | Self::MaxLength { message: m, .. }
            | Self::Pattern { message: m, .. }
            | Self::Min { message: m, .. }
            | Self::Max { message: m, .. }
            | Self::OneOf { message: m, .. }
            | Self::Custom { message: m, .. }
            | Self::MinItems { message: m, .. }
            | Self::MaxItems { message: m, .. } => {
                *m = Some(message.into());
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_length_constructor() {
        let rule = ValidationRule::min_length(3);
        match &rule {
            ValidationRule::MinLength { length, message } => {
                assert_eq!(*length, 3);
                assert!(message.is_none());
            }
            _ => panic!("expected MinLength"),
        }
    }

    #[test]
    fn max_length_constructor() {
        let rule = ValidationRule::max_length(255);
        match &rule {
            ValidationRule::MaxLength { length, message } => {
                assert_eq!(*length, 255);
                assert!(message.is_none());
            }
            _ => panic!("expected MaxLength"),
        }
    }

    #[test]
    fn pattern_constructor() {
        let rule = ValidationRule::pattern(r"^\w+@\w+\.\w+$");
        match &rule {
            ValidationRule::Pattern { pattern, message } => {
                assert_eq!(pattern, r"^\w+@\w+\.\w+$");
                assert!(message.is_none());
            }
            _ => panic!("expected Pattern"),
        }
    }

    #[test]
    fn min_constructor() {
        let rule = ValidationRule::min(0.0);
        match &rule {
            ValidationRule::Min { value, message } => {
                assert!((value - 0.0).abs() < f64::EPSILON);
                assert!(message.is_none());
            }
            _ => panic!("expected Min"),
        }
    }

    #[test]
    fn max_constructor() {
        let rule = ValidationRule::max(100.0);
        match &rule {
            ValidationRule::Max { value, message } => {
                assert!((value - 100.0).abs() < f64::EPSILON);
                assert!(message.is_none());
            }
            _ => panic!("expected Max"),
        }
    }

    #[test]
    fn range_creates_min_and_max() {
        let rules = ValidationRule::range(1.0, 65535.0);
        assert_eq!(rules.len(), 2);

        match &rules[0] {
            ValidationRule::Min { value, .. } => assert!((value - 1.0).abs() < f64::EPSILON),
            _ => panic!("first rule should be Min"),
        }
        match &rules[1] {
            ValidationRule::Max { value, .. } => assert!((value - 65535.0).abs() < f64::EPSILON),
            _ => panic!("second rule should be Max"),
        }
    }

    #[test]
    fn serde_min_length_round_trip() {
        let rule = ValidationRule::MinLength {
            length: 5,
            message: Some("too short".into()),
        };

        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"rule\":\"min_length\""));
        assert!(json.contains("\"length\":5"));
        assert!(json.contains("\"message\":\"too short\""));

        let deserialized: ValidationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn serde_pattern_round_trip() {
        let rule = ValidationRule::pattern(r"^[a-z]+$");
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: ValidationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn serde_one_of_round_trip() {
        let rule = ValidationRule::OneOf {
            values: vec![
                serde_json::json!("small"),
                serde_json::json!("medium"),
                serde_json::json!("large"),
            ],
            message: None,
        };

        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"rule\":\"one_of\""));

        let deserialized: ValidationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn serde_custom_round_trip() {
        let rule = ValidationRule::Custom {
            expression: "{{ $value > $json.min_age }}".into(),
            message: Some("must be above minimum age".into()),
        };

        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"rule\":\"custom\""));

        let deserialized: ValidationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn min_items_constructor() {
        let rule = ValidationRule::min_items(2);
        match &rule {
            ValidationRule::MinItems { count, message } => {
                assert_eq!(*count, 2);
                assert!(message.is_none());
            }
            _ => panic!("expected MinItems"),
        }
    }

    #[test]
    fn max_items_constructor() {
        let rule = ValidationRule::max_items(10);
        match &rule {
            ValidationRule::MaxItems { count, message } => {
                assert_eq!(*count, 10);
                assert!(message.is_none());
            }
            _ => panic!("expected MaxItems"),
        }
    }

    #[test]
    fn serde_min_items_round_trip() {
        let rule = ValidationRule::MinItems {
            count: 1,
            message: Some("need at least one".into()),
        };

        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"rule\":\"min_items\""));
        assert!(json.contains("\"count\":1"));

        let deserialized: ValidationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn serde_max_items_round_trip() {
        let rule = ValidationRule::max_items(50);
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"rule\":\"max_items\""));

        let deserialized: ValidationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn with_message_sets_custom_message() {
        let rule = ValidationRule::min_length(3).with_message("too short!");
        match &rule {
            ValidationRule::MinLength { length, message } => {
                assert_eq!(*length, 3);
                assert_eq!(message.as_deref(), Some("too short!"));
            }
            _ => panic!("expected MinLength"),
        }
    }

    #[test]
    fn with_message_works_on_all_variants() {
        let rules = [
            ValidationRule::min_length(1).with_message("a"),
            ValidationRule::max_length(10).with_message("b"),
            ValidationRule::pattern(".*").with_message("c"),
            ValidationRule::min(0.0).with_message("d"),
            ValidationRule::max(100.0).with_message("e"),
            ValidationRule::min_items(1).with_message("f"),
            ValidationRule::max_items(10).with_message("g"),
        ];

        let expected = ["a", "b", "c", "d", "e", "f", "g"];
        for (rule, msg) in rules.iter().zip(expected.iter()) {
            let json = serde_json::to_string(rule).unwrap();
            assert!(json.contains(msg), "expected message {msg} in {json}");
        }
    }

    #[test]
    fn optional_message_omitted_from_json() {
        let rule = ValidationRule::min_length(1);
        let json = serde_json::to_string(&rule).unwrap();
        assert!(!json.contains("message"));
    }

    #[test]
    fn equality_check() {
        let a = ValidationRule::min_length(10);
        let b = ValidationRule::min_length(10);
        let c = ValidationRule::min_length(20);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
