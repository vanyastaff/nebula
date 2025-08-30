//! Conditional validation using existing combinators
//! 
//! This module provides conditional validation by leveraging the existing
//! When combinator and other validation components.

use async_trait::async_trait;
use serde_json::Value;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity, ErrorCode};
use crate::traits::Validatable;
use crate::validators::combinators::When;
use crate::validators::basic::NotNull;

// ==================== Required If Validator ====================

/// Validator that makes a field required based on a condition
/// 
/// This validator uses the When combinator to implement required_if logic.
/// When the condition is met, the field must be present and valid.
#[derive(Debug, Clone)]
pub struct RequiredIf<V, C> {
    field_name: String,
    condition_field: String,
    condition: C,
    validator: V,
    name: String,
}

impl<V, C> RequiredIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    /// Create new required_if validator
    pub fn new(field_name: impl Into<String>, condition_field: impl Into<String>, condition: C, validator: V) -> Self {
        Self {
            field_name: field_name.into(),
            condition_field: condition_field.into(),
            condition,
            validator,
            name: "required_if".to_string(),
        }
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    
    /// Convert to When combinator for validation
    pub fn into_when(self) -> When<C, V> {
        When::new(self.condition, self.validator)
    }
}

#[async_trait]
impl<V, C> Validatable for RequiredIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // Get the parent object to access other fields
        let parent = self.get_parent_object(value)?;
        
        // Check if condition is met by validating the condition field
        let condition_met = self.condition.validate(parent.get(&self.condition_field).unwrap_or(&Value::Null)).await.is_ok();
        
        if condition_met {
            // Field is required, validate it
            if let Some(field_value) = parent.get(&self.field_name) {
                self.validator.validate(field_value).await
            } else {
                Err(ValidationError::new(
                    ErrorCode::Custom("field_required".to_string()),
                    format!("Field '{}' is required when condition is met", self.field_name)
                ))
            }
        } else {
            // Field is not required, skip validation
            Ok(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            self.name.clone(),
            format!("Field '{}' is required when '{}' meets condition", self.field_name, self.condition_field),
            crate::types::ValidatorCategory::Conditional,
        )
        .with_tags(vec!["conditional".to_string(), "required_if".to_string(), "cross_field".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Moderate
    }
}

// ==================== Forbidden If Validator ====================

/// Validator that forbids a field based on a condition
/// 
/// This validator uses the When combinator with Not logic to implement forbidden_if.
/// When the condition is met, the field must not be present.
#[derive(Debug, Clone)]
pub struct ForbiddenIf<V, C> {
    field_name: String,
    condition_field: String,
    condition: C,
    validator: V,
    name: String,
}

impl<V, C> ForbiddenIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    /// Create new forbidden_if validator
    pub fn new(field_name: impl Into<String>, condition_field: impl Into<String>, condition: C, validator: V) -> Self {
        Self {
            field_name: field_name.into(),
            condition_field: condition_field.into(),
            condition,
            validator,
            name: "forbidden_if".to_string(),
        }
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    
    /// Convert to When combinator for validation
    pub fn into_when(self) -> When<C, crate::validators::combinators::Not<V>> {
        use crate::validators::combinators::Not;
        When::new(self.condition, Not::new(self.validator))
    }
}

#[async_trait]
impl<V, C> Validatable for ForbiddenIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // Get the parent object to access other fields
        let parent = self.get_parent_object(value)?;
        
        // Check if condition is met by validating the condition field
        let condition_met = self.condition.validate(parent.get(&self.condition_field).unwrap_or(&Value::Null)).await.is_ok();
        
        if condition_met {
            // Field is forbidden, check if it's present
            if let Some(field_value) = parent.get(&self.field_name) {
                Err(ValidationError::new(
                    ErrorCode::Custom("field_forbidden".to_string()),
                    format!("Field '{}' is forbidden when condition is met", self.field_name)
                ))
            } else {
                // Field is not present, which is good
                Ok(())
            }
        } else {
            // Field is not forbidden, validate it if present
            if let Some(field_value) = parent.get(&self.field_name) {
                self.validator.validate(field_value).await
            } else {
                // Field is not present, which is fine
                Ok(())
            }
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            self.name.clone(),
            format!("Field '{}' is forbidden when '{}' meets condition", self.field_name, self.condition_field),
            crate::types::ValidatorCategory::Conditional,
        )
        .with_tags(vec!["conditional".to_string(), "forbidden_if".to_string(), "cross_field".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Moderate
    }
}

// ==================== Condition Validators ====================

/// Validator that checks if a field equals a specific value
/// 
/// This can be used as a condition in RequiredIf/ForbiddenIf validators.
#[derive(Debug, Clone)]
pub struct Equals<V> {
    expected_value: V,
    name: String,
}

impl<V> Equals<V>
where
    V: PartialEq + Send + Sync + Clone,
{
    /// Create new equals validator
    pub fn new(expected_value: V) -> Self {
        Self {
            expected_value,
            name: "equals".to_string(),
        }
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl<V> Validatable for Equals<V>
where
    V: PartialEq + Send + Sync + Clone + 'static,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This is a simplified implementation
        // In practice, you'd need proper Value to V conversion
        Err(ValidationError::new(
            ErrorCode::Custom("equals_not_implemented".to_string()),
            "Equals validator requires proper Value to T conversion"
        ))
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            self.name.clone(),
            format!("Value must equal {:?}", self.expected_value),
            crate::types::ValidatorCategory::Comparison,
        )
        .with_tags(vec!["equals".to_string(), "comparison".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

/// Validator that checks if a field value is in a set of allowed values
/// 
/// This can be used as a condition in RequiredIf/ForbiddenIf validators.
#[derive(Debug, Clone)]
pub struct In<V> {
    allowed_values: Vec<V>,
    name: String,
}

impl<V> In<V>
where
    V: PartialEq + Send + Sync + Clone,
{
    /// Create new in validator
    pub fn new(allowed_values: Vec<V>) -> Self {
        Self {
            allowed_values,
            name: "in".to_string(),
        }
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl<V> Validatable for In<V>
where
    V: PartialEq + Send + Sync + Clone + 'static,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This is a simplified implementation
        // In practice, you'd need proper Value to V conversion
        Err(ValidationError::new(
            ErrorCode::Custom("in_not_implemented".to_string()),
            "In validator requires proper Value to T conversion"
        ))
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            self.name.clone(),
            format!("Value must be one of {:?}", self.allowed_values),
            crate::types::ValidatorCategory::Comparison,
        )
        .with_tags(vec!["in".to_string(), "comparison".to_string(), "set".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

// ==================== Convenience Functions ====================

/// Create a required_if validator using When combinator
pub fn required_if<V, C>(field_name: impl Into<String>, condition_field: impl Into<String>, condition: C, validator: V) -> RequiredIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    RequiredIf::new(field_name, condition_field, condition, validator)
}

/// Create a forbidden_if validator using When combinator
pub fn forbidden_if<V, C>(field_name: impl Into<String>, condition_field: impl Into<String>, condition: C, validator: V) -> ForbiddenIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    ForbiddenIf::new(field_name, condition_field, condition, validator)
}

/// Create an equals condition validator
pub fn eq<V>(expected_value: V) -> Equals<V>
where
    V: PartialEq + Send + Sync + Clone,
{
    Equals::new(expected_value)
}

/// Create an in condition validator
pub fn in_values<V>(allowed_values: Vec<V>) -> In<V>
where
    V: PartialEq + Send + Sync + Clone,
{
    In::new(allowed_values)
}

// ==================== Helper Methods ====================

impl<V, C> RequiredIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    fn get_parent_object(&self, value: &Value) -> ValidationResult<&Value> {
        // This is a simplified implementation
        // In practice, you'd need to traverse up the object hierarchy
        // or pass the parent object context
        Ok(value)
    }
}

impl<V, C> ForbiddenIf<V, C>
where
    V: Validatable + Send + Sync + Clone,
    C: Validatable + Send + Sync + Clone,
{
    fn get_parent_object(&self, value: &Value) -> ValidationResult<&Value> {
        // Same as RequiredIf
        Ok(value)
    }
}

// ==================== Re-exports ====================

pub use RequiredIf as RequiredIf;
pub use ForbiddenIf as ForbiddenIf;
pub use Equals as Equals;
pub use In as In;
