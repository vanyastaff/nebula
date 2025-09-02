//! Context-aware validation traits

use async_trait::async_trait;
use serde_json::Value;
use crate::types::{ValidationResult, ValidatorMetadata};
use crate::context::ValidationContext;
use std::sync::Arc;

/// Validator that uses context for validation
#[async_trait]
pub trait ContextAwareValidator: Send + Sync {
    /// Validate with context
    async fn validate_with_context(
        &self,
        value: &Value,
        context: &ValidationContext,
    ) -> ValidationResult<()>;
    
    /// Get required context keys
    fn required_context_keys(&self) -> Vec<String> {
        Vec::new()
    }
    
    /// Check if context is valid
    fn is_context_valid(&self, context: &ValidationContext) -> bool {
        for key in self.required_context_keys() {
            if !context.contains_key(&key) {
                return false;
            }
        }
        true
    }
    
    /// Get metadata
    fn metadata(&self) -> ValidatorMetadata;
}

/// Context validator that requires specific context
#[derive(Debug, Clone)]
pub struct ContextValidator<F> {
    validator_fn: Arc<F>,
    required_keys: Vec<String>,
    metadata: ValidatorMetadata,
}

impl<F> ContextValidator<F>
where
    F: Fn(&Value, &ValidationContext) -> ValidationResult<()> + Send + Sync,
{
    /// Create new context validator
    pub fn new(
        name: impl Into<String>,
        validator_fn: F,
        required_keys: Vec<String>,
    ) -> Self {
        Self {
            validator_fn: Arc::new(validator_fn),
            required_keys,
            metadata: ValidatorMetadata::new(
                name.into(),
                "Context-aware validator",
                crate::types::ValidatorCategory::Custom,
            ),
        }
    }
}

#[async_trait]
impl<F> ContextAwareValidator for ContextValidator<F>
where
    F: Fn(&Value, &ValidationContext) -> ValidationResult<()> + Send + Sync,
{
    async fn validate_with_context(
        &self,
        value: &Value,
        context: &ValidationContext,
    ) -> ValidationResult<()> {
        if !self.is_context_valid(context) {
            return ValidationResult::error(crate::types::ValidationError::new(
                crate::types::ErrorCode::DependencyMissing,
                format!("Missing required context keys: {:?}", self.required_keys),
            ));
        }
        
        (self.validator_fn)(value, context)
    }
    
    fn required_context_keys(&self) -> Vec<String> {
        self.required_keys.clone()
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        self.metadata.clone()
    }
}

/// Cross-field validator using context
#[derive(Debug)]
pub struct CrossFieldValidator {
    field_paths: Vec<String>,
    validator_fn: Box<dyn Fn(&[(&str, &Value)]) -> ValidationResult<()> + Send + Sync>,
    metadata: ValidatorMetadata,
}

impl CrossFieldValidator {
    /// Create new cross-field validator
    pub fn new<F>(
        name: impl Into<String>,
        field_paths: Vec<String>,
        validator_fn: F,
    ) -> Self
    where
        F: Fn(&[(&str, &Value)]) -> ValidationResult<()> + Send + Sync + 'static,
    {
        Self {
            field_paths,
            validator_fn: Box::new(validator_fn),
            metadata: ValidatorMetadata::new(
                name.into(),
                "Cross-field validator",
                crate::types::ValidatorCategory::CrossField,
            ),
        }
    }
    
    /// Validate fields
    pub async fn validate_fields(&self, object: &Value) -> ValidationResult<()> {
        if let Value::Object(map) = object {
            let mut field_values = Vec::new();
            
            for path in &self.field_paths {
                if let Some(value) = map.get(path) {
                    field_values.push((path.as_str(), value));
                } else {
                    return ValidationResult::error(crate::types::ValidationError::new(
                        crate::types::ErrorCode::DependencyMissing,
                        format!("Missing field: {}", path),
                    ));
                }
            }
            
            (self.validator_fn)(&field_values)
        } else {
            ValidationResult::error(crate::types::ValidationError::new(
                crate::types::ErrorCode::TypeMismatch,
                "Expected object for cross-field validation",
            ))
        }
    }
}

/// Dependent field validator
#[derive(Debug)]
pub struct DependentFieldValidator {
    primary_field: String,
    dependent_fields: Vec<String>,
    dependency_type: DependencyType,
}

#[derive(Debug, Clone)]
pub enum DependencyType {
    /// All dependent fields must exist if primary exists
    Required,
    /// All dependent fields must not exist if primary exists
    Forbidden,
    /// At least one dependent field must exist if primary exists
    AtLeastOne,
    /// Exactly one dependent field must exist if primary exists
    ExactlyOne,
}

impl DependentFieldValidator {
    /// Create new dependent field validator
    pub fn new(
        primary_field: String,
        dependent_fields: Vec<String>,
        dependency_type: DependencyType,
    ) -> Self {
        Self {
            primary_field,
            dependent_fields,
            dependency_type,
        }
    }
    
    /// Validate dependencies
    pub async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Value::Object(map) = value {
            if map.contains_key(&self.primary_field) {
                let present_count = self.dependent_fields
                    .iter()
                    .filter(|field| map.contains_key(*field))
                    .count();
                
                match self.dependency_type {
                    DependencyType::Required => {
                        if present_count != self.dependent_fields.len() {
                            return ValidationResult::error(crate::types::ValidationError::new(
                                crate::types::ErrorCode::DependencyMissing,
                                format!("All dependent fields required when {} is present", self.primary_field),
                            ));
                        }
                    },
                    DependencyType::Forbidden => {
                        if present_count > 0 {
                            return ValidationResult::error(crate::types::ValidationError::new(
                                crate::types::ErrorCode::ConflictingFields,
                                format!("Dependent fields forbidden when {} is present", self.primary_field),
                            ));
                        }
                    },
                    DependencyType::AtLeastOne => {
                        if present_count == 0 {
                            return ValidationResult::error(crate::types::ValidationError::new(
                                crate::types::ErrorCode::DependencyMissing,
                                format!("At least one dependent field required when {} is present", self.primary_field),
                            ));
                        }
                    },
                    DependencyType::ExactlyOne => {
                        if present_count != 1 {
                            return ValidationResult::error(crate::types::ValidationError::new(
                                crate::types::ErrorCode::DependencyMissing,
                                format!("Exactly one dependent field required when {} is present", self.primary_field),
                            ));
                        }
                    },
                }
            }
            
            ValidationResult::success(())
        } else {
            ValidationResult::error(crate::types::ValidationError::new(
                crate::types::ErrorCode::TypeMismatch,
                "Expected object for dependent field validation",
            ))
        }
    }
}