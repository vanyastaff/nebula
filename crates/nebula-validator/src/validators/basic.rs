//! Basic validation operations
//! 
//! This module provides fundamental validators for common validation tasks.

use async_trait::async_trait;
use serde_json::Value;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity, ErrorCode};
use crate::traits::Validatable;

// ==================== Not Null Validator ====================

/// Validator that ensures a value is not null
/// 
/// This is a fundamental validator that checks if a value is present
/// and not null.
#[derive(Debug, Clone)]
pub struct NotNull {
    name: String,
}

impl NotNull {
    /// Create a new not null validator
    pub fn new() -> Self {
        Self {
            name: "not_null".to_string(),
        }
    }
    
    /// Set custom name for the validator
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl Default for NotNull {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for NotNull {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if value.is_null() {
            Err(ValidationError::new(
                ErrorCode::Custom("null_value".to_string()),
                "Value cannot be null"
            ))
        } else {
            Ok(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            self.name.clone(),
            "Value must not be null".to_string(),
            crate::types::ValidatorCategory::Basic,
        )
        .with_tags(vec!["basic".to_string(), "null".to_string(), "presence".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

// ==================== Convenience Functions ====================

/// Create a not null validator
pub fn not_null() -> NotNull {
    NotNull::new()
}

// ==================== Re-exports ====================

pub use NotNull as NotNull;

