//! String validation operations

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// MinLength validator - string must have minimum length
pub struct MinLength {
    min_length: usize,
}

impl MinLength {
    pub fn new(min_length: usize) -> Self {
        Self { min_length }
    }
}

#[async_trait]
impl Validatable for MinLength {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if s.len() >= self.min_length {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("String length {} is less than minimum {}", s.len(), self.min_length)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("min_length", "min_length", crate::ValidatorCategory::Basic)
            .with_description(format!("String minimum length: {}", self.min_length))
            .with_tags(vec!["string".to_string(), "length".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// MaxLength validator - string must have maximum length
pub struct MaxLength {
    max_length: usize,
}

impl MaxLength {
    pub fn new(max_length: usize) -> Self {
        Self { max_length }
    }
}

#[async_trait]
impl Validatable for MaxLength {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if s.len() <= self.max_length {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("String length {} is greater than maximum {}", s.len(), self.max_length)
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("max_length", "max_length", crate::ValidatorCategory::Basic)
            .with_description(format!("String maximum length: {}", self.max_length))
            .with_tags(vec!["string".to_string(), "length".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// Contains validator - string must contain the specified substring
pub struct Contains {
    substring: String,
}

impl Contains {
    pub fn new(substring: impl Into<String>) -> Self {
        Self { substring: substring.into() }
    }
}

#[async_trait]
impl Validatable for Contains {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if s.contains(&self.substring) {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("String does not contain '{}'", self.substring)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.substring.clone()))])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("contains", "contains", crate::ValidatorCategory::Basic)
            .with_description(format!("String must contain '{}'", self.substring))
            .with_tags(vec!["string".to_string(), "contains".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// StartsWith validator - string must start with the specified prefix
pub struct StartsWith {
    prefix: String,
}

impl StartsWith {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self { prefix: prefix.into() }
    }
}

#[async_trait]
impl Validatable for StartsWith {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if s.starts_with(&self.prefix) {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("String does not start with '{}'", self.prefix)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.prefix.clone()))])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("starts_with", "starts_with", crate::ValidatorCategory::Basic)
            .with_description(format!("String must start with '{}'", self.prefix))
            .with_tags(vec!["string".to_string(), "starts_with".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// EndsWith validator - string must end with the specified suffix
pub struct EndsWith {
    suffix: String,
}

impl EndsWith {
    pub fn new(suffix: impl Into<String>) -> Self {
        Self { suffix: suffix.into() }
    }
}

#[async_trait]
impl Validatable for EndsWith {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if s.ends_with(&self.suffix) {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("String does not end with '{}'", self.suffix)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.suffix.clone()))])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("ends_with", "ends_with", crate::ValidatorCategory::Basic)
            .with_description(format!("String must end with '{}'", self.suffix))
            .with_tags(vec!["string".to_string(), "ends_with".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// NonEmpty validator - string must not be empty or whitespace-only
pub struct NonEmpty;

#[async_trait]
impl Validatable for NonEmpty {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if !s.trim().is_empty() {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    "String cannot be empty or whitespace-only"
                ).with_actual_value(value.clone())])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("non_empty", "non_empty", crate::ValidatorCategory::Basic)
            .with_description("Non-empty string validation")
            .with_tags(vec!["string".to_string(), "length".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// StringLength validator - string must have exact length
pub struct StringLength {
    length: usize,
}

impl StringLength {
    pub fn new(length: usize) -> Self {
        Self { length }
    }
}

#[async_trait]
impl Validatable for StringLength {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if s.len() == self.length {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("String length {} does not equal expected length {}", s.len(), self.length)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(self.length)))])
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new("string_length", "string_length", crate::ValidatorCategory::Basic)
            .with_description(format!("String exact length: {}", self.length))
            .with_tags(vec!["string".to_string(), "length".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}
