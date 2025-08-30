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
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::NumberTooSmall,
                    format!("Value {} must be positive", n)
                ).with_actual_value(value.clone()))
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
            name: "positive",
            description: Some("Number must be positive"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "positive"],
        }
    }
}

/// Negative validator - number must be negative
pub struct Negative;

#[async_trait]
impl Validatable for Negative {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n < 0.0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} must be negative", n)
                ).with_actual_value(value.clone()))
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
            name: "negative",
            description: Some("Number must be negative"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "negative"],
        }
    }
}

/// Zero validator - number must be zero
pub struct Zero;

#[async_trait]
impl Validatable for Zero {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n == 0.0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} must be zero", n)
                ).with_actual_value(value.clone()))
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
            name: "zero",
            description: Some("Number must be zero"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "zero"],
        }
    }
}

/// NonZero validator - number must not be zero
pub struct NonZero;

#[async_trait]
impl Validatable for NonZero {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n != 0.0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    "Value must not be zero"
                ).with_actual_value(value.clone()))
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
            name: "non_zero",
            description: Some("Number must not be zero"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "non_zero"],
        }
    }
}

/// Integer validator - number must be an integer
pub struct Integer;

#[async_trait]
impl Validatable for Integer {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if value.is_i64() {
            Ok(())
        } else if let Some(n) = value.as_f64() {
            if n.fract() == 0.0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} is not an integer", n)
                ).with_actual_value(value.clone()))
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
            name: "integer",
            description: Some("Number must be an integer"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "integer"],
        }
    }
}

/// Even validator - number must be even
pub struct Even;

#[async_trait]
impl Validatable for Even {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n.fract() == 0.0 && (n as i64) % 2 == 0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} is not even", n)
                ).with_actual_value(value.clone()))
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
            name: "even",
            description: Some("Number must be even"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "even"],
        }
    }
}

/// Odd validator - number must be odd
pub struct Odd;

#[async_trait]
impl Validatable for Odd {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if n.fract() == 0.0 && (n as i64) % 2 != 0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} is not odd", n)
                ).with_actual_value(value.clone()))
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
            name: "odd",
            description: Some("Number must be odd"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "odd"],
        }
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
                return Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    "Divisor cannot be zero"
                ));
            }
            
            if n % self.divisor == 0.0 {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Value {} is not divisible by {}", n, self.divisor)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from_f64(self.divisor).unwrap())))
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
            name: "divisible_by",
            description: Some(&format!("Number must be divisible by {}", self.divisor)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["numeric", "divisible"],
        }
    }
}
