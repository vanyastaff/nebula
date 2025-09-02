//! WithValidator trait for adding validation capabilities to any type

use async_trait::async_trait;
use serde_json::Value;
use crate::core::{Valid, Invalid, Validated};
use crate::types::{ValidationResult, ValidatorMetadata, ValidationError};
use super::{Validatable, Validator};
use std::marker::PhantomData;

/// Trait for types that can be validated
#[async_trait]
pub trait WithValidator: Sized + Send + Sync {
    /// The validator type for this type
    type Validator: Validator<Self, Output = Self>;
    
    /// Get the default validator for this type
    fn default_validator() -> Self::Validator;
    
    /// Validate self with the default validator
    async fn validate(self) -> Result<Valid<Self>, Invalid<Self>> {
        Self::default_validator().validate(self).await
    }
    
    /// Validate self with a custom validator
    async fn validate_with<V>(self, validator: V) -> Result<Valid<Self>, Invalid<Self>>
    where
        V: Validator<Self, Output = Self>,
    {
        validator.validate(self).await
    }
    
    /// Try to validate and return self if valid, or error
    async fn ensure_valid(self) -> Result<Self, Vec<ValidationError>> {
        match self.validate().await {
            Ok(valid) => Ok(valid.into_value()),
            Err(invalid) => Err(invalid.errors().to_vec()),
        }
    }
    
    /// Check if self is valid without consuming
    async fn is_valid(&self) -> bool
    where
        Self: Clone,
    {
        self.clone().validate().await.is_ok()
    }
    
    /// Validate and transform to Validated enum
    async fn into_validated(self) -> Validated<Self> {
        match self.validate().await {
            Ok(valid) => Validated::Valid(valid),
            Err(invalid) => Validated::Invalid(invalid),
        }
    }
}

/// Extension trait for types that implement WithValidator
pub trait WithValidatorExt: WithValidator {
    /// Create a ValidatedType wrapper
    fn validated(self) -> ValidatedType<Self> {
        ValidatedType::new(self)
    }
    
    /// Validate with multiple validators
    async fn validate_all<V>(self, validators: Vec<V>) -> Result<Valid<Self>, Invalid<Self>>
    where
        V: Validator<Self, Output = Self>,
        Self: Clone,
    {
        let mut current = self;
        for validator in validators {
            match validator.validate(current.clone()).await {
                Ok(valid) => current = valid.into_value(),
                Err(invalid) => return Err(invalid),
            }
        }
        Ok(Valid::with_simple_proof(current, "validate_all"))
    }
    
    /// Validate with any of the validators (first success wins)
    async fn validate_any<V>(self, validators: Vec<V>) -> Result<Valid<Self>, Invalid<Self>>
    where
        V: Validator<Self, Output = Self>,
        Self: Clone,
    {
        let mut all_errors = Vec::new();
        
        for validator in validators {
            match validator.validate(self.clone()).await {
                Ok(valid) => return Ok(valid),
                Err(invalid) => all_errors.extend(invalid.errors().to_vec()),
            }
        }
        
        Err(Invalid::new(Some(self), all_errors))
    }
}

impl<T> WithValidatorExt for T where T: WithValidator {}

/// Wrapper type that ensures a value is validated
#[derive(Debug, Clone)]
pub struct ValidatedType<T> {
    value: Option<T>,
    validated: Option<Valid<T>>,
}

impl<T> ValidatedType<T>
where
    T: WithValidator,
{
    /// Create a new unvalidated wrapper
    pub fn new(value: T) -> Self {
        Self {
            value: Some(value),
            validated: None,
        }
    }
    
    /// Create from already validated value
    pub fn from_valid(valid: Valid<T>) -> Self {
        Self {
            value: None,
            validated: Some(valid),
        }
    }
    
    /// Ensure the value is validated
    pub async fn ensure_validated(&mut self) -> Result<&Valid<T>, Invalid<T>> {
        if self.validated.is_none() {
            if let Some(value) = self.value.take() {
                match value.validate().await {
                    Ok(valid) => self.validated = Some(valid),
                    Err(invalid) => return Err(invalid),
                }
            }
        }
        
        self.validated.as_ref().ok_or_else(|| {
            Invalid::without_value(vec![
                ValidationError::new(
                    crate::types::ErrorCode::InternalError,
                    "No value to validate",
                )
            ])
        })
    }
    
    /// Get the validated value
    pub async fn get(mut self) -> Result<T, Invalid<T>> {
        match self.ensure_validated().await {
            Ok(_) => Ok(self.validated.unwrap().into_value()),
            Err(invalid) => Err(invalid),
        }
    }
    
    /// Get reference to validated value
    pub async fn get_ref(&mut self) -> Result<&T, Invalid<T>> {
        self.ensure_validated().await.map(|valid| valid.value())
    }
}

/// Trait for types that can provide their own validator
pub trait SelfValidating {
    /// Validate self
    fn validate_self(&self) -> ValidationResult<()>;
    
    /// Get validation rules for self
    fn validation_rules(&self) -> Vec<Box<dyn Validatable>>;
    
    /// Get validation metadata
    fn validation_metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "self_validator",
            "Self-validating type",
            crate::types::ValidatorCategory::Custom,
        )
    }
}

/// Builder for validated types
pub struct ValidatedBuilder<T> {
    value: Option<T>,
    validators: Vec<Box<dyn Validator<T, Output = T>>>,
}

impl<T> ValidatedBuilder<T>
where
    T: Send + Sync + 'static,
{
    /// Create new builder
    pub fn new(value: T) -> Self {
        Self {
            value: Some(value),
            validators: Vec::new(),
        }
    }
    
    /// Add a validator
    pub fn with<V>(mut self, validator: V) -> Self
    where
        V: Validator<T, Output = T> + 'static,
    {
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Build and validate
    pub async fn build(self) -> Result<Valid<T>, Invalid<T>> {
        let mut current = self.value.unwrap();
        
        for validator in self.validators {
            match validator.validate(current).await {
                Ok(valid) => current = valid.into_value(),
                Err(invalid) => return Err(invalid),
            }
        }
        
        Ok(Valid::with_simple_proof(current, "validated_builder"))
    }
}

/// Trait for lazy validation
pub trait LazyValidator: Sized {
    /// Validator is created only when needed
    fn lazy_validator() -> Box<dyn Fn() -> Box<dyn Validator<Self, Output = Self>>>;
    
    /// Validate lazily
    async fn validate_lazy(self) -> Result<Valid<Self>, Invalid<Self>> {
        let validator = Self::lazy_validator()();
        validator.validate(self).await
    }
}

/// Trait for conditional validation based on the value
pub trait ConditionalValidator: Sized {
    /// Choose validator based on self
    fn choose_validator(&self) -> Box<dyn Validator<Self, Output = Self>>;
    
    /// Validate with chosen validator
    async fn validate_conditional(self) -> Result<Valid<Self>, Invalid<Self>>
    where
        Self: Clone,
    {
        let validator = self.choose_validator();
        validator.validate(self).await
    }
}

/// Auto-validation on access
#[derive(Debug)]
pub struct AutoValidated<T> {
    inner: T,
    validator: Box<dyn Validator<T, Output = T>>,
    cached_result: Option<ValidationResult<()>>,
}

impl<T> AutoValidated<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Create new auto-validated wrapper
    pub fn new<V>(value: T, validator: V) -> Self
    where
        V: Validator<T, Output = T> + 'static,
    {
        Self {
            inner: value,
            validator: Box::new(validator),
            cached_result: None,
        }
    }
    
    /// Get the value, validating if needed
    pub async fn get(&mut self) -> Result<&T, Vec<ValidationError>> {
        if self.cached_result.is_none() {
            match self.validator.validate(self.inner.clone()).await {
                Ok(_) => self.cached_result = Some(ValidationResult::success(())),
                Err(invalid) => {
                    self.cached_result = Some(ValidationResult::failure(invalid.errors().to_vec()));
                }
            }
        }
        
        match &self.cached_result {
            Some(result) if result.is_success() => Ok(&self.inner),
            Some(result) => Err(result.err().unwrap().clone()),
            None => unreachable!(),
        }
    }
    
    /// Invalidate cache
    pub fn invalidate(&mut self) {
        self.cached_result = None;
    }
    
    /// Update value and invalidate
    pub fn set(&mut self, value: T) {
        self.inner = value;
        self.cached_result = None;
    }
}