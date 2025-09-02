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
                ValidationResult::success(())
            } else {
                let expected_format = self.format.as_deref().unwrap_or("ISO 8601 (YYYY-MM-DD)");
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_date".to_string()),
                    format!("'{}' is not a valid date in format: {}", s, expected_format)
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
        let description = if let Some(ref format) = self.format {
            format!("String must be a valid date in format: {}", format)
        } else {
            "String must be a valid date in ISO 8601 format".to_string()
        };
        
        crate::ValidatorMetadata::new(
            "date",
            "date",
            crate::ValidatorCategory::Format,
        )
        .with_description(description)
        .with_tags(vec!["format".to_string(), "date".to_string()])
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
                ValidationResult::success(())
            } else {
                let expected_format = self.format.as_deref().unwrap_or("ISO 8601 (YYYY-MM-DDTHH:MM:SS)");
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_datetime".to_string()),
                    format!("'{}' is not a valid datetime in format: {}", s, expected_format)
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
        let description = if let Some(ref format) = self.format {
            format!("String must be a valid datetime in format: {}", format)
        } else {
            "String must be a valid datetime in ISO 8601 format".to_string()
        };
        
        crate::ValidatorMetadata::new(
            "datetime",
            "datetime",
            crate::ValidatorCategory::Format,
        )
        .with_description(description)
        .with_tags(vec!["format".to_string(), "datetime".to_string()])
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
                ValidationResult::success(())
            } else {
                let expected_format = self.format.as_deref().unwrap_or("HH:MM:SS");
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_time".to_string()),
                    format!("'{}' is not a valid time in format: {}", s, expected_format)
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
        let description = if let Some(ref format) = self.format {
            format!("String must be a valid time in format: {}", format)
        } else {
            "String must be a valid time in HH:MM:SS format".to_string()
        };
        
        crate::ValidatorMetadata::new(
            "time",
            "time",
            crate::ValidatorCategory::Format,
        )
        .with_description(description)
        .with_tags(vec!["format".to_string(), "time".to_string()])
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
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_base64".to_string()),
                    "'{}' is not valid base64"
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
        crate::ValidatorMetadata::new(
            "base64",
            "base64",
            crate::ValidatorCategory::Format,
        )
        .with_description("String must be valid base64")
        .with_tags(vec!["format".to_string(), "base64".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
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
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_hex".to_string()),
                    "'{}' is not valid hexadecimal"
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
        crate::ValidatorMetadata::new(
            "hex",
            "hex",
            crate::ValidatorCategory::Format,
        )
        .with_description("String must be valid hexadecimal")
        .with_tags(vec!["format".to_string(), "hex".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
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
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_json".to_string()),
                    "'{}' is not valid JSON"
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
        crate::ValidatorMetadata::new(
            "json",
            "json",
            crate::ValidatorCategory::Format,
        )
        .with_description("String must be valid JSON")
        .with_tags(vec!["format".to_string(), "json".to_string()])
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
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_xml".to_string()),
                    "'{}' is not valid XML"
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
        crate::ValidatorMetadata::new(
            "xml",
            "xml",
            crate::ValidatorCategory::Format,
        )
        .with_description("String must be valid XML")
        .with_tags(vec!["format".to_string(), "xml".to_string()])
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
                    ValidationResult::success(())
                } else {
                    ValidationResult::failure(vec![ValidationError::new(
                        ErrorCode::Custom("invalid_credit_card".to_string()),
                        "'{}' is not a valid credit card number (checksum failed)"
                    ).with_actual_value(value.clone())])
                }
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_credit_card".to_string()),
                    "'{}' is not a valid credit card number"
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
        crate::ValidatorMetadata::new(
            "credit_card",
            "credit_card",
            crate::ValidatorCategory::Format,
        )
        .with_description("String must be a valid credit card number")
        .with_tags(vec!["format".to_string(), "credit_card".to_string()])
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
                ValidationResult::success(())
            } else {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::Custom("invalid_phone_number".to_string()),
                    "'{}' is not a valid phone number"
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
        crate::ValidatorMetadata::new(
            "phone_number",
            "phone_number",
            crate::ValidatorCategory::Format,
        )
        .with_description("String must be a valid phone number")
        .with_tags(vec!["format".to_string(), "phone_number".to_string()])
    }
}
