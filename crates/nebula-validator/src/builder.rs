//! Fluent Builder API for Nebula Validator
//! 
//! This module provides a type-safe, fluent builder interface for creating
//! validators with compile-time guarantees.

use std::marker::PhantomData;
use crate::traits::Validatable;
use crate::validators::{
    string::StringLength,
    numeric::Numeric,
    regex::Regex,
    format::{Email, Url, Uuid, Ip},
    basic::NotNull,
};

// ==================== Marker Types for Type Safety ====================

/// Marker type for unvalidated state
pub struct Unvalidated;

/// Marker type for validated state
pub struct Validated;

/// Marker type for string validation
pub struct StringValidation;

/// Marker type for numeric validation
pub struct NumericValidation;

/// Marker type for collection validation
pub struct CollectionValidation;

// ==================== Base Builder ====================

/// Base validation builder with phantom types for type safety
pub struct ValidationBuilder<T = (), S = Unvalidated> {
    _phantom: PhantomData<(T, S)>,
    validators: Vec<Box<dyn Validatable>>,
}

impl ValidationBuilder {
    /// Create a new validation builder
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
            validators: Vec::new(),
        }
    }
    
    /// Start building a string validator
    pub fn string() -> ValidationBuilder<String, Unvalidated> {
        ValidationBuilder::<String, Unvalidated>::new()
    }
    
    /// Start building a numeric validator
    pub fn numeric() -> ValidationBuilder<f64, Unvalidated> {
        ValidationBuilder::<f64, Unvalidated>::new()
    }
    
    /// Start building a collection validator
    pub fn collection() -> ValidationBuilder<Vec<()>, Unvalidated> {
        ValidationBuilder::<Vec<()>, Unvalidated>::new()
    }
    
    /// Start building a custom validator
    pub fn custom() -> ValidationBuilder<(), Unvalidated> {
        ValidationBuilder::<(), Unvalidated>::new()
    }
}

impl<T> ValidationBuilder<T, Unvalidated> {
    /// Add a validator to the chain
    fn add_validator<V: Validatable + 'static>(mut self, validator: V) -> Self {
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Build the final validator
    pub fn build(self) -> ValidationBuilder<T, Validated> {
        ValidationBuilder {
            _phantom: PhantomData,
            validators: self.validators,
        }
    }
}

// ==================== String Validation Builder ====================

impl ValidationBuilder<String, Unvalidated> {
    /// Set minimum length for string validation
    pub fn min_length(mut self, min: usize) -> Self {
        let validator = StringLength::new(min, None);
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Set maximum length for string validation
    pub fn max_length(mut self, max: usize) -> Self {
        let validator = StringLength::new(None, max);
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Set both minimum and maximum length
    pub fn length_range(mut self, min: usize, max: usize) -> Self {
        let validator = StringLength::new(Some(min), Some(max));
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add regex pattern validation
    pub fn pattern(mut self, regex: &str) -> Result<Self, regex::Error> {
        let validator = Regex::new(regex)?;
        self.validators.push(Box::new(validator));
        Ok(self)
    }
    
    /// Add email format validation
    pub fn email(mut self) -> Self {
        let validator = Email::new();
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add URL format validation
    pub fn url(mut self) -> Self {
        let validator = Url::new();
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add UUID format validation
    pub fn uuid(mut self) -> Self {
        let validator = Uuid::new();
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add IP address format validation
    pub fn ip(mut self) -> Self {
        let validator = Ip::new();
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add required field validation
    pub fn required(mut self) -> Self {
        let validator = NotNull::new();
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add custom validation function
    pub fn custom<F>(mut self, name: &str, validator_fn: F) -> Self
    where
        F: Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    {
        let validator = CustomStringValidator::new(name.to_string(), validator_fn);
        self.validators.push(Box::new(validator));
        self
    }
}

// ==================== Numeric Validation Builder ====================

impl ValidationBuilder<f64, Unvalidated> {
    /// Set minimum value for numeric validation
    pub fn min(mut self, min: f64) -> Self {
        let validator = Numeric::new(Some(min), None);
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Set maximum value for numeric validation
    pub fn max(mut self, max: f64) -> Self {
        let validator = Numeric::new(None, Some(max));
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Set both minimum and maximum values
    pub fn range(mut self, min: f64, max: f64) -> Self {
        let validator = Numeric::new(Some(min), Some(max));
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add required field validation
    pub fn required(mut self) -> Self {
        let validator = NotNull::new();
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add custom validation function
    pub fn custom<F>(mut self, name: &str, validator_fn: F) -> Self
    where
        F: Fn(f64) -> Result<(), String> + Send + Sync + 'static,
    {
        let validator = CustomNumericValidator::new(name.to_string(), validator_fn);
        self.validators.push(Box::new(validator));
        self
    }
}

// ==================== Collection Validation Builder ====================

impl<T: 'static> ValidationBuilder<Vec<T>, Unvalidated> {
    /// Set minimum length for collection validation
    pub fn min_length(mut self, min: usize) -> Self {
        let validator = CollectionLengthValidator::new().min_length(min);
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Set maximum length for collection validation
    pub fn max_length(mut self, max: usize) -> Self {
        let validator = CollectionLengthValidator::new().max_length(max);
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Set both minimum and maximum length
    pub fn length_range(mut self, min: usize, max: usize) -> Self {
        let validator = CollectionLengthValidator::new()
            .min_length(min)
            .max_length(max);
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add required field validation
    pub fn required(mut self) -> Self {
        let validator = NotNull::new();
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add custom validation function
    pub fn custom<F>(mut self, name: &str, validator_fn: F) -> Self
    where
        F: Fn(&[T]) -> Result<(), String> + Send + Sync + 'static,
    {
        let validator = CustomCollectionValidator::new(name.to_string(), validator_fn);
        self.validators.push(Box::new(validator));
        self
    }
}

// ==================== Custom Validators ====================

/// Custom string validator
struct CustomStringValidator<F> {
    name: String,
    validator_fn: F,
}

impl<F> CustomStringValidator<F>
where
    F: Fn(&str) -> Result<(), String> + Send + Sync + 'static,
{
    fn new(name: String, validator_fn: F) -> Self {
        Self { name, validator_fn }
    }
}

#[async_trait::async_trait]
impl<F> Validatable for CustomStringValidator<F>
where
    F: Fn(&str) -> Result<(), String> + Send + Sync + 'static,
{
    async fn validate(&self, value: &crate::Value) -> crate::ValidationResult<()> {
        if let Some(s) = value.as_str() {
            match (self.validator_fn)(s) {
                Ok(()) => crate::ValidationResult::success(()),
                Err(msg) => crate::ValidationResult::failure(vec![crate::ValidationError::new(
                    crate::ErrorCode::CustomValidation,
                    msg,
                )]),
            }
        } else {
            crate::ValidationResult::failure(vec![crate::ValidationError::new(
                crate::ErrorCode::InvalidType,
                "Value must be a string",
            )])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            self.name.clone(),
            format!("Custom String Validator: {}", self.name),
            crate::ValidatorCategory::Custom,
        )
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// Custom numeric validator
struct CustomNumericValidator<F> {
    name: String,
    validator_fn: F,
}

impl<F> CustomNumericValidator<F>
where
    F: Fn(f64) -> Result<(), String> + Send + Sync + 'static,
{
    fn new(name: String, validator_fn: F) -> Self {
        Self { name, validator_fn }
    }
}

#[async_trait::async_trait]
impl<F> Validatable for CustomNumericValidator<F>
where
    F: Fn(f64) -> Result<(), String> + Send + Sync + 'static,
{
    async fn validate(&self, value: &crate::Value) -> crate::ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            match (self.validator_fn)(n) {
                Ok(()) => crate::ValidationResult::success(()),
                Err(msg) => crate::ValidationResult::failure(vec![crate::ValidationError::new(
                    crate::ErrorCode::CustomValidation,
                    msg,
                )]),
            }
        } else {
            crate::ValidationResult::failure(vec![crate::ValidationError::new(
                crate::ErrorCode::InvalidType,
                "Value must be a number",
            )])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            self.name.clone(),
            format!("Custom Numeric Validator: {}", self.name),
            crate::ValidatorCategory::Custom,
        )
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

/// Custom collection validator
struct CustomCollectionValidator<F, T> {
    name: String,
    validator_fn: F,
    _phantom: PhantomData<T>,
}

impl<F, T> CustomCollectionValidator<F, T>
where
    F: Fn(&[T]) -> Result<(), String> + Send + Sync + 'static,
{
    fn new(name: String, validator_fn: F) -> Self {
        Self { 
            name, 
            validator_fn,
            _phantom: PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<F, T> Validatable for CustomCollectionValidator<F, T>
where
    F: Fn(&[T]) -> Result<(), String> + Send + Sync + 'static,
{
    async fn validate(&self, value: &crate::Value) -> crate::ValidationResult<()> {
        if let Some(arr) = value.as_array() {
            // Note: This is a simplified implementation
            // In practice, you'd need to handle type conversion properly
            match (self.validator_fn)(&[]) {
                Ok(()) => Ok(()),
                Err(msg) => Err(vec![crate::ValidationError::new(
                    crate::ErrorCode::CustomValidation,
                    msg,
                )]),
            }
        } else {
            Err(vec![crate::ValidationError::new(
                crate::ErrorCode::InvalidType,
                "Value must be an array",
            )])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            self.name.clone(),
            format!("Custom Collection Validator: {}", self.name),
            crate::ValidatorCategory::Custom,
        )
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

// ==================== Collection Length Validator ====================

/// Collection length validator
struct CollectionLengthValidator {
    min_length: Option<usize>,
    max_length: Option<usize>,
}

impl CollectionLengthValidator {
    fn new() -> Self {
        Self {
            min_length: None,
            max_length: None,
        }
    }
    
    fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }
    
    fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }
}

#[async_trait::async_trait]
impl Validatable for CollectionLengthValidator {
    async fn validate(&self, value: &crate::Value) -> crate::ValidationResult<()> {
        if let Some(arr) = value.as_array() {
            let len = arr.len();
            
            if let Some(min) = self.min_length {
                if len < min {
                    return Err(vec![crate::ValidationError::new(
                        crate::ErrorCode::TooShort,
                        format!("Collection must have at least {} items", min),
                    )]);
                }
            }
            
            if let Some(max) = self.max_length {
                if len > max {
                    return Err(vec![crate::ValidationError::new(
                        crate::ErrorCode::TooLong,
                        format!("Collection must have at most {} items", max),
                    )]);
                }
            }
            
            Ok(())
        } else {
            Err(vec![crate::ValidationError::new(
                crate::ErrorCode::InvalidType,
                "Value must be an array",
            )])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "collection_length".to_string(),
            "Collection Length Validator".to_string(),
            crate::ValidatorCategory::Collection,
        )
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        crate::ValidationComplexity::Simple
    }
}

// ==================== Composite Validator ====================

/// Composite validator that chains multiple validators
pub struct CompositeValidator {
    validators: Vec<Box<dyn Validatable>>,
}

impl CompositeValidator {
    /// Create a new composite validator
    pub fn new(validators: Vec<Box<dyn Validatable>>) -> Self {
        Self { validators }
    }
    
    /// Add a validator to the chain
    pub fn add_validator<V: Validatable + 'static>(mut self, validator: V) -> Self {
        self.validators.push(Box::new(validator));
        self
    }
}

#[async_trait::async_trait]
impl Validatable for CompositeValidator {
    async fn validate(&self, value: &crate::Value) -> crate::ValidationResult<()> {
        for validator in &self.validators {
            if let Err(errors) = validator.validate(value).await {
                return Err(errors);
            }
        }
        Ok(())
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "composite".to_string(),
            "Composite Validator".to_string(),
            crate::ValidatorCategory::Composite,
        )
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        if self.validators.len() <= 1 {
            crate::ValidationComplexity::Simple
        } else if self.validators.len() <= 5 {
            crate::ValidationComplexity::Moderate
        } else {
            crate::ValidationComplexity::Complex
        }
    }
}

// ==================== Builder Extensions ====================

impl<T> ValidationBuilder<T, Validated> {
    /// Get the composite validator
    pub fn into_validator(self) -> CompositeValidator {
        CompositeValidator::new(self.validators)
    }
    
    /// Validate a value using this builder
    pub async fn validate(&self, value: &crate::Value) -> crate::ValidationResult<()> {
        let validator = CompositeValidator::new(self.validators.clone());
        validator.validate(value).await
    }
}

// ==================== Convenience Functions ====================

/// Create a string validator builder
pub fn string() -> ValidationBuilder<String, Unvalidated> {
    ValidationBuilder::string()
}

/// Create a numeric validator builder
pub fn numeric() -> ValidationBuilder<f64, Unvalidated> {
    ValidationBuilder::numeric()
}

/// Create a collection validator builder
pub fn collection() -> ValidationBuilder<Vec<()>, Unvalidated> {
    ValidationBuilder::collection()
}

/// Create a custom validator builder
pub fn custom() -> ValidationBuilder<(), Unvalidated> {
    ValidationBuilder::custom()
}

// ==================== Examples ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    
    #[tokio::test]
    async fn test_string_validation_builder() {
        let validator = string()
            .min_length(3)
            .max_length(20)
            .email()
            .required()
            .build();
        
        // Valid email
        let valid_email = Value::String("test@example.com".to_string());
        let result = validator.validate(&valid_email).await;
        assert!(result.is_ok());
        
        // Invalid email (too short)
        let invalid_email = Value::String("ab@c".to_string());
        let result = validator.validate(&invalid_email).await;
        assert!(result.is_err());
    }
    
    #[tokio::test]
    async fn test_numeric_validation_builder() {
        let validator = numeric()
            .min(0.0)
            .max(100.0)
            .required()
            .build();
        
        // Valid number
        let valid_number = Value::Number(serde_json::Number::from_f64(50.0).unwrap());
        let result = validator.validate(&valid_number).await;
        assert!(result.is_ok());
        
        // Invalid number (out of range)
        let invalid_number = Value::Number(serde_json::Number::from_f64(150.0).unwrap());
        let result = validator.validate(&invalid_number).await;
        assert!(result.is_err());
    }
    
    #[tokio::test]
    async fn test_collection_validation_builder() {
        let validator = collection()
            .min_length(1)
            .max_length(10)
            .required()
            .build();
        
        // Valid collection
        let valid_collection = Value::Array(vec![Value::String("item".to_string())]);
        let result = validator.validate(&valid_collection).await;
        assert!(result.is_ok());
        
        // Invalid collection (empty)
        let invalid_collection = Value::Array(vec![]);
        let result = validator.validate(&invalid_collection).await;
        assert!(result.is_err());
    }
    
    #[tokio::test]
    async fn test_custom_validation() {
        let validator = string()
            .custom("no_spaces", |s| {
                if s.contains(' ') {
                    Err("String cannot contain spaces".to_string())
                } else {
                    Ok(())
                }
            })
            .build();
        
        // Valid string
        let valid_string = Value::String("nospaces".to_string());
        let result = validator.validate(&valid_string).await;
        assert!(result.is_ok());
        
        // Invalid string
        let invalid_string = Value::String("has spaces".to_string());
        let result = validator.validate(&invalid_string).await;
        assert!(result.is_err());
    }
}
