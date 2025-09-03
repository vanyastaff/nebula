//! Main Validatable trait for validators working with serde_json::Value

use async_trait::async_trait;
use serde_json::Value;
use crate::types::{
    ValidationResult, ValidatorMetadata, ValidationComplexity,
    ValidationConfig, ValidatorId,
};
use crate::context::ValidationContext;
/// Main trait for validators that work with `serde_json::Value`
#[async_trait]
pub trait Validatable: Send + Sync {
    /// Validate a JSON value
    async fn validate(&self, value: &Value) -> ValidationResult<()>;
    
    /// Get validator metadata
    fn metadata(&self) -> ValidatorMetadata;
    
    /// Get validation complexity
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
    
    /// Check if the validator supports caching
    fn is_cacheable(&self) -> bool {
        true
    }
    
    /// Get cache key for the value (if cacheable)
    fn cache_key(&self, value: &Value) -> Option<String> {
        if self.is_cacheable() {
            Some(format!("{}:{:?}", self.metadata().id, value))
        } else {
            None
        }
    }
    
    /// Validate with context (default implementation ignores context)
    async fn validate_with_context(
        &self,
        value: &Value,
        _context: &ValidationContext,
    ) -> ValidationResult<()> {
        self.validate(value).await
    }
    
    /// Validate with configuration
    async fn validate_with_config(
        &self,
        value: &Value,
        _config: &ValidationConfig,
    ) -> ValidationResult<()> {
        self.validate(value).await
    }
    
    /// Get validator ID
    fn id(&self) -> ValidatorId {
        self.metadata().id.clone()
    }
    
    /// Get validator name
    fn name(&self) -> String {
        self.metadata().name.clone()
    }
    
    /// Check if this validator is compatible with a value type
    fn accepts(&self, value: &Value) -> bool {
        // By default, accept all value types
        // Validators can override this for type checking
        true
    }
    
    /// Estimate validation time for planning
    fn estimate_time_ms(&self, value: &Value) -> u64 {
        let size = estimate_value_size(value);
        match self.complexity() {
            ValidationComplexity::Trivial => 1,
            ValidationComplexity::Simple => size as u64,
            ValidationComplexity::Moderate => (size as f64 * (size as f64).log2()) as u64,
            ValidationComplexity::Complex => (size * size) as u64,
            ValidationComplexity::VeryComplex => (size * size * size) as u64,
        }
    }
}

/// Helper function to estimate the size of a JSON value
fn estimate_value_size(value: &Value) -> usize {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => 1,
        Value::String(s) => s.len(),
        Value::Array(arr) => arr.len(),
        Value::Object(obj) => obj.len(),
    }
}

/// Trait for cloneable validators
pub trait ValidatableClone: Validatable {
    /// Clone the validator into a box
    fn clone_box(&self) -> Box<dyn Validatable>;
}

impl<T> ValidatableClone for T
where
    T: Validatable + Clone + 'static,
{
    fn clone_box(&self) -> Box<dyn Validatable> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn Validatable> {
    fn clone(&self) -> Self {
        // This requires validators to implement ValidatableClone
        // which is automatically implemented for Clone types
        panic!("Cannot clone Box<dyn Validatable> - use ValidatableClone trait")
    }
}

// Blanket implementation for Box<dyn Validatable>
#[async_trait]
impl Validatable for Box<dyn Validatable> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        self.as_ref().validate(value).await
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        self.as_ref().metadata()
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.as_ref().complexity()
    }
    
    fn is_cacheable(&self) -> bool {
        self.as_ref().is_cacheable()
    }
}