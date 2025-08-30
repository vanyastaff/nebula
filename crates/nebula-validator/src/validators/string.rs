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
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::StringTooShort,
                    format!("String length {} is less than minimum {}", s.len(), self.min_length)
                ).with_actual_value(value.clone()))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "min_length",
            description: Some(&format!("String minimum length: {}", self.min_length)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["string", "length"],
        }
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
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::StringTooLong,
                    format!("String length {} is greater than maximum {}", s.len(), self.max_length)
                ).with_actual_value(value.clone()))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "max_length",
            description: Some(&format!("String maximum length: {}", self.max_length)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["string", "length"],
        }
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
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("String does not contain '{}'", self.substring)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.substring.clone())))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "contains",
            description: Some(&format!("String must contain '{}'", self.substring)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["string", "contains"],
        }
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
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("String does not start with '{}'", self.prefix)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.prefix.clone())))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "starts_with",
            description: Some(&format!("String must start with '{}'", self.prefix)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["string", "starts_with"],
        }
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
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("String does not end with '{}'", self.suffix)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.suffix.clone())))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "ends_with",
            description: Some(&format!("String must end with '{}'", self.suffix)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["string", "ends_with"],
        }
    }
}

/// NonEmpty validator - string must not be empty or whitespace-only
pub struct NonEmpty;

#[async_trait]
impl Validatable for NonEmpty {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if !s.trim().is_empty() {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::StringEmpty,
                    "String cannot be empty or whitespace-only"
                ).with_actual_value(value.clone()))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "non_empty",
            description: Some("Non-empty string validation"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["string", "empty"],
        }
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
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("String length {} does not equal expected length {}", s.len(), self.length)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(self.length))))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "string_length",
            description: Some(&format!("String exact length: {}", self.length)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["string", "length"],
        }
    }
}
