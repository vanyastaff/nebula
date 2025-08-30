//! Comparison validation operations

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// Equals validator - value must equal the expected value
pub struct Equals {
    expected: Value,
}

impl Equals {
    pub fn new(expected: Value) -> Self {
        Self { expected }
    }
}

#[async_trait]
impl Validatable for Equals {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if value == &self.expected {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                "Value does not equal expected value"
            ).with_actual_value(value.clone())
             .with_expected_value(self.expected.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "equals",
            description: Some("Value must equal expected value"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "equals"],
        }
    }
}

/// NotEquals validator - value must not equal the expected value
pub struct NotEquals {
    expected: Value,
}

impl NotEquals {
    pub fn new(expected: Value) -> Self {
        Self { expected }
    }
}

#[async_trait]
impl Validatable for NotEquals {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if value != &self.expected {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                "Value must not equal expected value"
            ).with_actual_value(value.clone())
             .with_expected_value(self.expected.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "not_equals",
            description: Some("Value must not equal expected value"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "not_equals"],
        }
    }
}

/// GreaterThan validator - numeric value must be greater than the threshold
pub struct GreaterThan {
    threshold: f64,
}

impl GreaterThan {
    pub fn new(threshold: f64) -> Self {
        Self { threshold }
    }
}

#[async_trait]
impl Validatable for GreaterThan {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n > self.threshold {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::NumberTooSmall,
                    format!("Value {} must be greater than {}", n, self.threshold)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from_f64(self.threshold).unwrap())))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "greater_than",
            description: Some(&format!("Value must be greater than {}", self.threshold)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "greater_than"],
        }
    }
}

/// LessThan validator - numeric value must be less than the threshold
pub struct LessThan {
    threshold: f64,
}

impl LessThan {
    pub fn new(threshold: f64) -> Self {
        Self { threshold }
    }
}

#[async_trait]
impl Validatable for LessThan {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n < self.threshold {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::NumberTooLarge,
                    format!("Value {} must be less than {}", n, self.threshold)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from_f64(self.threshold).unwrap())))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "less_than",
            description: Some(&format!("Value must be less than {}", self.threshold)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "less_than"],
        }
    }
}

/// GreaterThanOrEqual validator - numeric value must be greater than or equal to the threshold
pub struct GreaterThanOrEqual {
    threshold: f64,
}

impl GreaterThanOrEqual {
    pub fn new(threshold: f64) -> Self {
        Self { threshold }
    }
}

#[async_trait]
impl Validatable for GreaterThanOrEqual {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n >= self.threshold {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::NumberTooSmall,
                    format!("Value {} must be greater than or equal to {}", n, self.threshold)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from_f64(self.threshold).unwrap())))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "greater_than_or_equal",
            description: Some(&format!("Value must be greater than or equal to {}", self.threshold)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "greater_than_or_equal"],
        }
    }
}

/// LessThanOrEqual validator - numeric value must be less than or equal to the threshold
pub struct LessThanOrEqual {
    threshold: f64,
}

impl LessThanOrEqual {
    pub fn new(threshold: f64) -> Self {
        Self { threshold }
    }
}

#[async_trait]
impl Validatable for LessThanOrEqual {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n <= self.threshold {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::NumberTooLarge,
                    format!("Value {} must be less than or equal to {}", n, self.threshold)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from_f64(self.threshold).unwrap())))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "less_than_or_equal",
            description: Some(&format!("Value must be less than or equal to {}", self.threshold)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "less_than_or_equal"],
        }
    }
}

/// Between validator - numeric value must be between min and max (inclusive)
pub struct Between {
    min: f64,
    max: f64,
}

impl Between {
    pub fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }
}

#[async_trait]
impl Validatable for Between {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n >= self.min && n <= self.max {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} must be between {} and {}", n, self.min, self.max)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Object(serde_json::Map::from_iter(vec![
                     ("min".to_string(), Value::Number(serde_json::Number::from_f64(self.min).unwrap())),
                     ("max".to_string(), Value::Number(serde_json::Number::from_f64(self.max).unwrap()))
                 ]))))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "between",
            description: Some(&format!("Value must be between {} and {}", self.min, self.max)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "between"],
        }
    }
}

/// NotBetween validator - numeric value must not be between min and max (inclusive)
pub struct NotBetween {
    min: f64,
    max: f64,
}

impl NotBetween {
    pub fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }
}

#[async_trait]
impl Validatable for NotBetween {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n < self.min || n > self.max {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} must not be between {} and {}", n, self.min, self.max)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Object(serde_json::Map::from_iter(vec![
                     ("min".to_string(), Value::Number(serde_json::Number::from_f64(self.min).unwrap())),
                     ("max".to_string(), Value::Number(serde_json::Number::from_f64(self.max).unwrap()))
                 ]))))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "not_between",
            description: Some(&format!("Value must not be between {} and {}", self.min, self.max)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["comparison", "not_between"],
        }
    }
}

// Legacy aliases for backward compatibility
pub type MinValue = GreaterThanOrEqual;
pub type MaxValue = LessThanOrEqual;
