//! Simplified builder pattern for validation composition

use crate::core::{Validator, ValidatorExt, ValidationContext, Valid, Invalid};
use async_trait::async_trait;
use bon::Builder;

// ==================== Validation Builder ====================

/// Simple builder for creating validation chains
#[derive(Builder)]
pub struct ValidationBuilder<V: Validator> {
    validator: V,
    name: Option<String>,
}

impl<V: Validator> ValidationBuilder<V> {
    /// Create a new validation builder
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            name: None,
        }
    }

    /// Set a custom name for this validation
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add another validator with AND logic
    pub fn and<Other: Validator>(self, other: Other) -> ValidationBuilder<impl Validator> {
        let name = self.name;
        let new_validator = self.validator.and(other);

        ValidationBuilder {
            validator: new_validator,
            name,
        }
    }

    /// Add another validator with OR logic
    pub fn or<Other: Validator>(self, other: Other) -> ValidationBuilder<impl Validator> {
        let name = self.name;
        let new_validator = self.validator.or(other);

        ValidationBuilder {
            validator: new_validator,
            name,
        }
    }

    /// Negate this validator
    pub fn not(self) -> ValidationBuilder<impl Validator> {
        let name = self.name;
        let new_validator = self.validator.not();

        ValidationBuilder {
            validator: new_validator,
            name,
        }
    }

    /// Build the final validator
    pub fn build(self) -> BuiltValidator<V> {
        let validator_name = self.validator.name().to_string();
        BuiltValidator {
            validator: self.validator,
            name: self.name.unwrap_or(validator_name),
        }
    }
}

// ==================== Built Validator ====================

/// A validator created through the builder pattern
pub struct BuiltValidator<V: Validator> {
    validator: V,
    name: String,
}

#[async_trait]
impl<V: Validator> Validator for BuiltValidator<V> {
    async fn validate(&self, value: &nebula_value::Value, context: Option<&ValidationContext>) -> Result<Valid<()>, Invalid<()>> {
        self.validator.validate(value, context).await
            .map_err(|invalid| invalid.with_validator_name(&self.name))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> Option<&str> {
        self.validator.description()
    }
}

// ==================== Convenience Functions ====================

/// Create a validation builder
pub fn validate<V: Validator>(validator: V) -> ValidationBuilder<V> {
    ValidationBuilder::new(validator)
}

/// Create a validation builder with optional name using builder pattern
#[bon::builder]
pub fn build_validator<V: Validator>(
    validator: V,
    name: Option<String>
) -> ValidationBuilder<V> {
    ValidationBuilder { validator, name }
}