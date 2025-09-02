//! Numeric validation operations

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// Positive validator - number must be positive
pub struct Positive;

#[async_trait]
impl Validatable for Positive {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n > 0.0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Value {} must be positive", n)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("positive", "positive", crate::ValidatorCategory::Basic)
            .with_description("Number must be positive")
            .with_tags(vec!["numeric".to_string(), "positive".to_string()])
    }
}

/// Negative validator - number must be negative
pub struct Negative;

#[async_trait]
impl Validatable for Negative {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n < 0.0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Value {} must be negative", n)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("negative", "negative", crate::ValidatorCategory::Basic)
            .with_description("Number must be negative")
            .with_tags(vec!["numeric".to_string(), "negative".to_string()])
    }
}

/// Zero validator - number must be zero
pub struct Zero;

#[async_trait]
impl Validatable for Zero {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n == 0.0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Value {} must be zero", n)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("zero", "zero", crate::ValidatorCategory::Basic)
            .with_description("Number must be zero")
            .with_tags(vec!["numeric".to_string(), "zero".to_string()])
    }
}

/// NonZero validator - number must not be zero
pub struct NonZero;

#[async_trait]
impl Validatable for NonZero {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n != 0.0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    "Value must not be zero"
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("non_zero", "non_zero", crate::ValidatorCategory::Basic)
            .with_description("Number must not be zero")
            .with_tags(vec!["numeric".to_string(), "non_zero".to_string()])
    }
}

/// Integer validator - number must be an integer
pub struct Integer;

#[async_trait]
impl Validatable for Integer {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if value.is_i64() {
            ValidationResult::success(())
        } else if let Some(n) = value.as_f64() {
            if n.fract() == 0.0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Value {} is not an integer", n)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("integer", "integer", crate::ValidatorCategory::Basic)
            .with_description("Number must be an integer")
            .with_tags(vec!["numeric".to_string(), "integer".to_string()])
    }
}

/// Even validator - number must be even
pub struct Even;

#[async_trait]
impl Validatable for Even {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n.fract() == 0.0 && (n as i64) % 2 == 0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Value {} is not even", n)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("even", "even", crate::ValidatorCategory::Basic)
            .with_description("Number must be even")
            .with_tags(vec!["numeric".to_string(), "even".to_string()])
    }
}

/// Odd validator - number must be odd
pub struct Odd;

#[async_trait]
impl Validatable for Odd {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n.fract() == 0.0 && (n as i64) % 2 != 0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Value {} is not odd", n)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("odd", "odd", crate::ValidatorCategory::Basic)
            .with_description("Number must be odd")
            .with_tags(vec!["numeric".to_string(), "odd".to_string()])
    }
}

/// DivisibleBy validator - number must be divisible by the divisor
pub struct DivisibleBy {
    divisor: f64,
}

impl DivisibleBy {
    pub fn new(divisor: f64) -> Self {
        Self { divisor }
    }
}

#[async_trait]
impl Validatable for DivisibleBy {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if self.divisor == 0.0 {
                return ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    "Divisor cannot be zero"
                )]);
            }
            
            if n % self.divisor == 0.0 {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Value {} is not divisible by {}", n, self.divisor)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from_f64(self.divisor).unwrap()))])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("divisible_by", "divisible_by", crate::ValidatorCategory::Basic)
            .with_description(format!("Number must be divisible by {}", self.divisor))
            .with_tags(vec!["numeric".to_string(), "divisible".to_string()])
    }
}
