//! Function-based validator for custom validation logic

use crate::core::{ConfigResult, ConfigValidator};
use async_trait::async_trait;
use std::sync::Arc;

/// Function-based validator using a closure
pub struct FunctionValidator {
    /// Validation function
    validator_fn: Arc<dyn Fn(&serde_json::Value) -> ConfigResult<()> + Send + Sync>,
    /// Optional schema
    schema: Option<serde_json::Value>,
}

impl std::fmt::Debug for FunctionValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionValidator")
            .field("validator_fn", &"<function>")
            .field("schema", &self.schema.is_some())
            .finish()
    }
}

impl FunctionValidator {
    /// Create a new function validator
    pub fn new<F>(validator_fn: F) -> Self
    where
        F: Fn(&serde_json::Value) -> ConfigResult<()> + Send + Sync + 'static,
    {
        Self {
            validator_fn: Arc::new(validator_fn),
            schema: None,
        }
    }

    /// Create with a schema
    pub fn with_schema<F>(validator_fn: F, schema: serde_json::Value) -> Self
    where
        F: Fn(&serde_json::Value) -> ConfigResult<()> + Send + Sync + 'static,
    {
        Self {
            validator_fn: Arc::new(validator_fn),
            schema: Some(schema),
        }
    }
}

#[async_trait]
impl ConfigValidator for FunctionValidator {
    async fn validate(&self, data: &serde_json::Value) -> ConfigResult<()> {
        (self.validator_fn)(data)
    }

    fn schema(&self) -> Option<serde_json::Value> {
        self.schema.clone()
    }
}

/// Builder for creating complex function validators
pub struct FunctionValidatorBuilder {
    validators: Vec<Box<dyn Fn(&serde_json::Value) -> ConfigResult<()> + Send + Sync>>,
    schema: Option<serde_json::Value>,
}

impl FunctionValidatorBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
            schema: None,
        }
    }

    /// Add a validation function
    pub fn add_validator<F>(mut self, f: F) -> Self
    where
        F: Fn(&serde_json::Value) -> ConfigResult<()> + Send + Sync + 'static,
    {
        self.validators.push(Box::new(f));
        self
    }

    /// Add a field validator
    pub fn validate_field<F>(self, field: &str, validator: F) -> Self
    where
        F: Fn(&serde_json::Value) -> ConfigResult<()> + Send + Sync + 'static,
    {
        let field = field.to_string();
        self.add_validator(move |data| {
            if let Some(value) = data.get(&field) {
                validator(value)
            } else {
                Ok(())
            }
        })
    }

    /// Require a field to be present
    pub fn require_field(self, field: &str) -> Self {
        let field = field.to_string();
        self.add_validator(move |data| {
            if data.get(&field).is_none() {
                Err(crate::core::ConfigError::validation_error(
                    format!("Required field '{}' is missing", field),
                    Some(field.clone()),
                ))
            } else {
                Ok(())
            }
        })
    }

    /// Set schema
    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Build the validator
    pub fn build(self) -> FunctionValidator {
        let validators = self.validators;
        let combined_fn = move |data: &serde_json::Value| -> ConfigResult<()> {
            for validator in &validators {
                validator(data)?;
            }
            Ok(())
        };

        FunctionValidator {
            validator_fn: Arc::new(combined_fn),
            schema: self.schema,
        }
    }
}

impl Default for FunctionValidatorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_function_validator() {
        let validator = FunctionValidator::new(|data| {
            if data.get("required_field").is_none() {
                Err(crate::core::ConfigError::validation_error(
                    "Missing required field",
                    None,
                ))
            } else {
                Ok(())
            }
        });

        let valid_data = json!({ "required_field": "value" });
        assert!(validator.validate(&valid_data).await.is_ok());

        let invalid_data = json!({ "other_field": "value" });
        assert!(validator.validate(&invalid_data).await.is_err());
    }

    #[tokio::test]
    async fn test_function_validator_builder() {
        let validator = FunctionValidatorBuilder::new()
            .require_field("name")
            .require_field("version")
            .validate_field("port", |value| {
                if let Some(port) = value.as_u64() {
                    if port > 0 && port < 65536 {
                        Ok(())
                    } else {
                        Err(crate::core::ConfigError::validation_error(
                            "Port must be between 1 and 65535",
                            Some("port".to_string()),
                        ))
                    }
                } else {
                    Err(crate::core::ConfigError::validation_error(
                        "Port must be a number",
                        Some("port".to_string()),
                    ))
                }
            })
            .build();

        let valid_data = json!({
            "name": "app",
            "version": "1.0.0",
            "port": 8080
        });
        assert!(validator.validate(&valid_data).await.is_ok())
    }
}