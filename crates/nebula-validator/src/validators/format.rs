//! Format validation operations

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// Date validator - string must be a valid date
pub struct Date {
    format: Option<String>,
}

impl Date {
    pub fn new() -> Self {
        Self { format: None }
    }

    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }
}

impl Default for Date {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for Date {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Try to parse as ISO 8601 date by default
            let result = if let Some(ref format) = self.format {
                chrono::NaiveDate::parse_from_str(s, format)
            } else {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .or_else(|_| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
                    .or_else(|_| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
            };
            
            if result.is_ok() {
                Ok(())
            } else {
                let expected_format = self.format.as_deref().unwrap_or("ISO 8601 (YYYY-MM-DD)");
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("'{}' is not a valid date in format: {}", s, expected_format)
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
        let description = if let Some(ref format) = self.format {
            format!("String must be a valid date in format: {}", format)
        } else {
            "String must be a valid date in ISO 8601 format".to_string()
        };
        
        crate::ValidatorMetadata {
            name: "date",
            description: Some(&description),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "date"],
        }
    }
}

/// DateTime validator - string must be a valid datetime
pub struct DateTime {
    format: Option<String>,
}

impl DateTime {
    pub fn new() -> Self {
        Self { format: None }
    }

    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }
}

impl Default for DateTime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for DateTime {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Try to parse as ISO 8601 datetime by default
            let result = if let Some(ref format) = self.format {
                chrono::NaiveDateTime::parse_from_str(s, format)
            } else {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f"))
            };
            
            if result.is_ok() {
                Ok(())
            } else {
                let expected_format = self.format.as_deref().unwrap_or("ISO 8601 (YYYY-MM-DDTHH:MM:SS)");
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("'{}' is not a valid datetime in format: {}", s, expected_format)
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
        let description = if let Some(ref format) = self.format {
            format!("String must be a valid datetime in format: {}", format)
        } else {
            "String must be a valid datetime in ISO 8601 format".to_string()
        };
        
        crate::ValidatorMetadata {
            name: "datetime",
            description: Some(&description),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "datetime"],
        }
    }
}

/// Time validator - string must be a valid time
pub struct Time {
    format: Option<String>,
}

impl Time {
    pub fn new() -> Self {
        Self { format: None }
    }

    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for Time {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Try to parse as time by default
            let result = if let Some(ref format) = self.format {
                chrono::NaiveTime::parse_from_str(s, format)
            } else {
                chrono::NaiveTime::parse_from_str(s, "%H:%M:%S")
                    .or_else(|_| chrono::NaiveTime::parse_from_str(s, "%H:%M"))
                    .or_else(|_| chrono::NaiveTime::parse_from_str(s, "%H:%M:%S%.f"))
            };
            
            if result.is_ok() {
                Ok(())
            } else {
                let expected_format = self.format.as_deref().unwrap_or("HH:MM:SS");
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    format!("'{}' is not a valid time in format: {}", s, expected_format)
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
        let description = if let Some(ref format) = self.format {
            format!("String must be a valid time in format: {}", format)
        } else {
            "String must be a valid time in HH:MM:SS format".to_string()
        };
        
        crate::ValidatorMetadata {
            name: "time",
            description: Some(&description),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "time"],
        }
    }
}

/// Base64 validator - string must be valid base64
pub struct Base64;

#[async_trait]
impl Validatable for Base64 {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Check if string is valid base64
            if base64::decode(s).is_ok() {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    "'{}' is not valid base64"
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
            name: "base64",
            description: Some("String must be valid base64"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "base64"],
        }
    }
}

/// Hex validator - string must be valid hexadecimal
pub struct Hex;

#[async_trait]
impl Validatable for Hex {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Check if string contains only valid hex characters
            if s.chars().all(|c| c.is_ascii_hexdigit()) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    "'{}' is not valid hexadecimal"
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
            name: "hex",
            description: Some("String must be valid hexadecimal"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "hex"],
        }
    }
}

/// Json validator - string must be valid JSON
pub struct Json;

#[async_trait]
impl Validatable for Json {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Check if string can be parsed as JSON
            if serde_json::from_str::<Value>(s).is_ok() {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    "'{}' is not valid JSON"
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
            name: "json",
            description: Some("String must be valid JSON"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "json"],
        }
    }
}

/// Xml validator - string must be valid XML
pub struct Xml;

#[async_trait]
impl Validatable for Xml {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Basic XML validation - check for opening and closing tags
            if s.trim().starts_with('<') && s.trim().ends_with('>') {
                // This is a very basic check. In production, you might want to use a proper XML parser
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    "'{}' is not valid XML"
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
            name: "xml",
            description: Some("String must be valid XML"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "xml"],
        }
    }
}

/// CreditCard validator - string must be a valid credit card number
pub struct CreditCard;

#[async_trait]
impl Validatable for CreditCard {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Remove spaces and dashes
            let cleaned = s.replace([' ', '-'], "");
            
            // Check if it's all digits and has reasonable length
            if cleaned.chars().all(|c| c.is_ascii_digit()) && (13..=19).contains(&cleaned.len()) {
                // Basic Luhn algorithm check
                let sum: u32 = cleaned
                    .chars()
                    .rev()
                    .enumerate()
                    .map(|(i, c)| {
                        let digit = c.to_digit(10).unwrap();
                        if i % 2 == 1 {
                            let doubled = digit * 2;
                            if doubled > 9 { doubled - 9 } else { doubled }
                        } else {
                            digit
                        }
                    })
                    .sum();
                
                if sum % 10 == 0 {
                    Ok(())
                } else {
                    Err(ValidationError::new(
                        ErrorCode::PatternMismatch,
                        "'{}' is not a valid credit card number (checksum failed)"
                    ).with_actual_value(value.clone()))
                }
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    "'{}' is not a valid credit card number"
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
            name: "credit_card",
            description: Some("String must be a valid credit card number"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "credit_card"],
        }
    }
}

/// PhoneNumber validator - string must be a valid phone number
pub struct PhoneNumber;

#[async_trait]
impl Validatable for PhoneNumber {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            // Remove common separators
            let cleaned = s.replace([' ', '-', '(', ')', '+'], "");
            
            // Check if it's all digits and has reasonable length
            if cleaned.chars().all(|c| c.is_ascii_digit()) && (7..=15).contains(&cleaned.len()) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::PatternMismatch,
                    "'{}' is not a valid phone number"
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
            name: "phone_number",
            description: Some("String must be a valid phone number"),
            category: crate::ValidatorCategory::Format,
            tags: vec!["format", "phone_number"],
        }
    }
}
