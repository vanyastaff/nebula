//! Range validation for various data types
//! 
//! This module provides validators for checking if values fall within specified ranges,
//! including numeric ranges, date ranges, string length ranges, and custom range types.

use async_trait::async_trait;
use serde_json::Value;
use std::cmp::{PartialOrd, PartialEq};
use std::fmt::Display;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity, ErrorCode};
use crate::traits::Validatable;

// ==================== Numeric Range Validator ====================

/// Validator for numeric ranges (integers, floats)
/// 
/// This validator checks if a numeric value falls within a specified range.
/// Supports inclusive and exclusive bounds, and can be configured for different numeric types.
#[derive(Debug, Clone)]
pub struct NumericRange<T> {
    min: Option<T>,
    max: Option<T>,
    min_inclusive: bool,
    max_inclusive: bool,
    name: String,
}

impl<T> NumericRange<T>
where
    T: PartialOrd + PartialEq + Display + Send + Sync + Clone,
{
    /// Create a new numeric range validator
    pub fn new() -> Self {
        Self {
            min: None,
            max: None,
            min_inclusive: true,
            max_inclusive: true,
            name: "numeric_range".to_string(),
        }
    }
    
    /// Set minimum value (inclusive)
    pub fn min(mut self, min: T) -> Self {
        self.min = Some(min);
        self
    }
    
    /// Set maximum value (inclusive)
    pub fn max(mut self, max: T) -> Self {
        self.max = Some(max);
        self
    }
    
    /// Set minimum value (exclusive)
    pub fn min_exclusive(mut self, min: T) -> Self {
        self.min = Some(min);
        self.min_inclusive = false;
        self
    }
    
    /// Set maximum value (exclusive)
    pub fn max_exclusive(mut self, max: T) -> Self {
        self.max = Some(max);
        self.max_inclusive = false;
        self
    }
    
    /// Set both min and max values
    pub fn range(mut self, min: T, max: T) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    
    /// Validate a numeric value
    fn validate_numeric(&self, value: &T) -> ValidationResult<()> {
        // Check minimum bound
        if let Some(min) = &self.min {
            let is_valid = if self.min_inclusive {
                *value >= *min
            } else {
                *value > *min
            };
            
            if !is_valid {
                let op = if self.min_inclusive { ">=" } else { ">" };
                return Err(ValidationError::new(
                    ErrorCode::Custom("value_too_small".to_string()),
                    format!("Value {} must be {} {}", value, op, min)
                ));
            }
        }
        
        // Check maximum bound
        if let Some(max) = &self.max {
            let is_valid = if self.max_inclusive {
                *value <= *max
            } else {
                *value < *max
            };
            
            if !is_valid {
                let op = if self.max_inclusive { "<=" } else { "<" };
                return Err(ValidationError::new(
                    ErrorCode::Custom("value_too_large".to_string()),
                    format!("Value {} must be {} {}", value, op, max)
                ));
            }
        }
        
        Ok(())
    }
}

#[async_trait]
impl<T> Validatable for NumericRange<T>
where
    T: PartialOrd + PartialEq + Display + Send + Sync + Clone,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        match value {
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    self.validate_numeric(&i)
                } else if let Some(f) = n.as_f64() {
                    self.validate_numeric(&f)
                } else {
                    Err(ValidationError::new(
                        ErrorCode::Custom("invalid_numeric".to_string()),
                        "Value is not a valid number"
                    ))
                }
            }
            _ => Err(ValidationError::new(
                ErrorCode::Custom("type_mismatch".to_string()),
                "Expected numeric value"
            ))
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut description = String::new();
        
        if let Some(min) = &self.min {
            let op = if self.min_inclusive { ">=" } else { ">" };
            description.push_str(&format!("Value {} {}", op, min));
        }
        
        if let Some(max) = &self.max {
            if !description.is_empty() {
                description.push_str(" and ");
            }
            let op = if self.max_inclusive { "<=" } else { "<" };
            description.push_str(&format!("{} {}", op, max));
        }
        
        if description.is_empty() {
            description = "Any numeric value".to_string();
        }
        
        ValidatorMetadata::new(
            self.name.clone(),
            description,
            crate::types::ValidatorCategory::Numeric,
        )
        .with_tags(vec!["range".to_string(), "numeric".to_string(), "bounds".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

// ==================== String Length Range Validator ====================

/// Validator for string length ranges
/// 
/// This validator checks if a string's length falls within a specified range.
/// Useful for validating minimum and maximum character counts.
#[derive(Debug, Clone)]
pub struct StringLengthRange {
    min_length: Option<usize>,
    max_length: Option<usize>,
    name: String,
}

impl StringLengthRange {
    /// Create a new string length range validator
    pub fn new() -> Self {
        Self {
            min_length: None,
            max_length: None,
            name: "string_length_range".to_string(),
        }
    }
    
    /// Set minimum length
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }
    
    /// Set maximum length
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }
    
    /// Set both min and max length
    pub fn range(mut self, min: usize, max: usize) -> Self {
        self.min_length = Some(min);
        self.max_length = Some(max);
        self
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl Validatable for StringLengthRange {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let string_value = match value {
            Value::String(s) => s,
            _ => return Err(ValidationError::new(
                ErrorCode::Custom("type_mismatch".to_string()),
                "Expected string value"
            ))
        };
        
        let length = string_value.len();
        
        // Check minimum length
        if let Some(min_len) = self.min_length {
            if length < min_len {
                return Err(ValidationError::new(
                    ErrorCode::Custom("string_too_short".to_string()),
                    format!("String length {} is less than minimum {}", length, min_len)
                ));
            }
        }
        
        // Check maximum length
        if let Some(max_len) = self.max_length {
            if length > max_len {
                return Err(ValidationError::new(
                    ErrorCode::Custom("string_too_long".to_string()),
                    format!("String length {} exceeds maximum {}", length, max_len)
                ));
            }
        }
        
        Ok(())
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut description = String::new();
        
        if let Some(min) = self.min_length {
            description.push_str(&format!("Length >= {}", min));
        }
        
        if let Some(max) = self.max_length {
            if !description.is_empty() {
                description.push_str(" and ");
            }
            description.push_str(&format!("<= {}", max));
        }
        
        if description.is_empty() {
            description = "Any string length".to_string();
        }
        
        ValidatorMetadata::new(
            self.name.clone(),
            description,
            crate::types::ValidatorCategory::String,
        )
        .with_tags(vec!["range".to_string(), "string".to_string(), "length".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

// ==================== Array Length Range Validator ====================

/// Validator for array length ranges
/// 
/// This validator checks if an array's length falls within a specified range.
/// Useful for validating minimum and maximum element counts.
#[derive(Debug, Clone)]
pub struct ArrayLengthRange {
    min_length: Option<usize>,
    max_length: Option<usize>,
    name: String,
}

impl ArrayLengthRange {
    /// Create a new array length range validator
    pub fn new() -> Self {
        Self {
            min_length: None,
            max_length: None,
            name: "array_length_range".to_string(),
        }
    }
    
    /// Set minimum length
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }
    
    /// Set maximum length
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }
    
    /// Set both min and max length
    pub fn range(mut self, min: usize, max: usize) -> Self {
        self.min_length = Some(min);
        self.max_length = Some(max);
        self
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl Validatable for ArrayLengthRange {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let array_value = match value {
            Value::Array(arr) => arr,
            _ => return Err(ValidationError::new(
                ErrorCode::Custom("type_mismatch".to_string()),
                "Expected array value"
            ))
        };
        
        let length = array_value.len();
        
        // Check minimum length
        if let Some(min_len) = self.min_length {
            if length < min_len {
                return Err(ValidationError::new(
                    ErrorCode::Custom("array_too_short".to_string()),
                    format!("Array length {} is less than minimum {}", length, min_len)
                ));
            }
        }
        
        // Check maximum length
        if let Some(max_len) = self.max_length {
            if length > max_len {
                return Err(ValidationError::new(
                    ErrorCode::Custom("array_too_long".to_string()),
                    format!("Array length {} exceeds maximum {}", length, max_len)
                ));
            }
        }
        
        Ok(())
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let mut description = String::new();
        
        if let Some(min) = self.min_length {
            description.push_str(&format!("Length >= {}", min));
        }
        
        if let Some(max) = self.max_length {
            if !description.is_empty() {
                description.push_str(" and ");
            }
            description.push_str(&format!("<= {}", max));
        }
        
        if description.is_empty() {
            description = "Any array length".to_string();
        }
        
        ValidatorMetadata::new(
            self.name.clone(),
            description,
            crate::types::ValidatorCategory::Collection,
        )
        .with_tags(vec!["range".to_string(), "array".to_string(), "length".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

// ==================== Custom Range Validator ====================

/// Generic range validator for custom types
/// 
/// This validator allows you to define custom range logic for any type
/// that implements the required traits.
#[derive(Debug, Clone)]
pub struct CustomRange<T, F> {
    validator: F,
    name: String,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, F> CustomRange<T, F>
where
    F: Fn(&T) -> ValidationResult<()> + Send + Sync + Clone,
{
    /// Create a new custom range validator
    pub fn new(validator: F) -> Self {
        Self {
            validator,
            name: "custom_range".to_string(),
            _phantom: std::marker::PhantomData,
        }
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl<T, F> Validatable for CustomRange<T, F>
where
    F: Fn(&T) -> ValidationResult<()> + Send + Sync + Clone,
    T: 'static,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This is a simplified implementation - in practice you'd need
        // to implement proper deserialization from Value to T
        Err(ValidationError::new(
            ErrorCode::Custom("custom_range_not_implemented".to_string()),
            "Custom range validation requires proper Value to T conversion"
        ))
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            self.name.clone(),
            "Custom range validation".to_string(),
            crate::types::ValidatorCategory::Custom,
        )
        .with_tags(vec!["range".to_string(), "custom".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Moderate
    }
}

// ==================== Range Builder ====================

/// Builder for creating range validators
/// 
/// This builder provides a fluent interface for creating various types of range validators.
#[derive(Debug, Clone)]
pub struct RangeBuilder {
    name: Option<String>,
}

impl RangeBuilder {
    /// Create a new range builder
    pub fn new() -> Self {
        Self { name: None }
    }
    
    /// Set custom name for the range validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    
    /// Build a numeric range validator
    pub fn numeric<T>(self) -> NumericRange<T>
    where
        T: PartialOrd + PartialEq + Display + Send + Sync + Clone,
    {
        let mut validator = NumericRange::new();
        if let Some(name) = self.name {
            validator = validator.with_name(name);
        }
        validator
    }
    
    /// Build a string length range validator
    pub fn string_length(self) -> StringLengthRange {
        let mut validator = StringLengthRange::new();
        if let Some(name) = self.name {
            validator = validator.with_name(name);
        }
        validator
    }
    
    /// Build an array length range validator
    pub fn array_length(self) -> ArrayLengthRange {
        let mut validator = ArrayLengthRange::new();
        if let Some(name) = self.name {
            validator = validator.with_name(name);
        }
        validator
    }
}

impl Default for RangeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Convenience Functions ====================

/// Create a numeric range validator
pub fn numeric_range<T>() -> NumericRange<T>
where
    T: PartialOrd + PartialEq + Display + Send + Sync + Clone,
{
    NumericRange::new()
}

/// Create a string length range validator
pub fn string_length_range() -> StringLengthRange {
    StringLengthRange::new()
}

/// Create an array length range validator
pub fn array_length_range() -> ArrayLengthRange {
    ArrayLengthRange::new()
}

/// Create a range builder
pub fn range() -> RangeBuilder {
    RangeBuilder::new()
}

// ==================== Re-exports ====================

pub use NumericRange as Numeric;
pub use StringLengthRange as StringLength;
pub use ArrayLengthRange as ArrayLength;
pub use CustomRange as Custom;
pub use RangeBuilder as Builder;
