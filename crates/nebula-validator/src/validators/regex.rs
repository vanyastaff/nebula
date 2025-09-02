//! Regular expression validation operations

use async_trait::async_trait;
use serde_json::Value;
use regex::Regex;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// Pattern validator - string must match the regex pattern
pub struct Pattern {
    regex: Regex,
    pattern: String,
}

impl Pattern {
    pub fn new(pattern: impl Into<String>) -> Result<Self, regex::Error> {
        let pattern_str = pattern.into();
        let regex = Regex::new(&pattern_str)?;
        Ok(Self { regex, pattern: pattern_str })
    }
}

#[async_trait]
impl Validatable for Pattern {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if self.regex.is_match(s) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("String '{}' does not match pattern '{}'", s, self.pattern)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.pattern.clone())))
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
            name: "pattern",
            description: Some(&format!("String must match pattern: {}", self.pattern)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["regex", "pattern"],
        }
    }
}

/// NotPattern validator - string must not match the regex pattern
pub struct NotPattern {
    regex: Regex,
    pattern: String,
}

impl NotPattern {
    pub fn new(pattern: impl Into<String>) -> Result<Self, regex::Error> {
        let pattern_str = pattern.into();
        let regex = Regex::new(&pattern_str)?;
        Ok(Self { regex, pattern: pattern_str })
    }
}

#[async_trait]
impl Validatable for NotPattern {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if !self.regex.is_match(s) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("String '{}' matches forbidden pattern '{}'", s, self.pattern)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::String(self.pattern.clone())))
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
            name: "not_pattern",
            description: Some(&format!("String must not match pattern: {}", self.pattern)),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["regex", "not_pattern"],
        }
    }
}

/// Email validator - string must be a valid email address
pub struct Email;

#[async_trait]
impl Validatable for Email {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Simple email regex pattern
            let email_regex = Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();
            
            if email_regex.is_match(s) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("'{}' is not a valid email address", s)
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
            name: "email",
            description: Some("String must be a valid email address"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["regex", "email", "format"],
        }
    }
}

/// Url validator - string must be a valid URL
pub struct Url;

#[async_trait]
impl Validatable for Url {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Simple URL regex pattern
            let url_regex = Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap();
            
            if url_regex.is_match(s) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("'{}' is not a valid URL", s)
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
            name: "url",
            description: Some("String must be a valid URL"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["regex", "url", "format"],
        }
    }
}

/// Uuid validator - string must be a valid UUID
pub struct Uuid;

#[async_trait]
impl Validatable for Uuid {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // UUID regex pattern
            let uuid_regex = Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$").unwrap();
            
            if uuid_regex.is_match(s) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("'{}' is not a valid UUID", s)
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
            name: "uuid",
            description: Some("String must be a valid UUID"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["regex", "uuid", "format"],
        }
    }
}

/// IpAddress validator - string must be a valid IP address
pub struct IpAddress;

#[async_trait]
impl Validatable for IpAddress {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // IPv4 regex pattern
            let ipv4_regex = Regex::new(r"^(\d{1,3}\.){3}\d{1,3}$").unwrap();
            // IPv6 regex pattern (simplified)
            let ipv6_regex = Regex::new(r"^([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}$").unwrap();
            
            if ipv4_regex.is_match(s) || ipv6_regex.is_match(s) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("'{}' is not a valid IP address", s)
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
            name: "ip_address",
            description: Some("String must be a valid IP address"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["regex", "ip", "format"],
        }
    }
}

/// Alphanumeric validator - string must contain only alphanumeric characters
pub struct Alphanumeric;

#[async_trait]
impl Validatable for Alphanumeric {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            let alphanumeric_regex = Regex::new(r"^[a-zA-Z0-9]+$").unwrap();
            
            if alphanumeric_regex.is_match(s) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    "String must contain only alphanumeric characters"
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
            name: "alphanumeric",
            description: Some("String must contain only alphanumeric characters"),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["regex", "alphanumeric"],
        }
    }
}
