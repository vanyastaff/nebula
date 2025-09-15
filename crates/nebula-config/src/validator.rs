//! Configuration validation

use crate::{ConfigError, ConfigResult};
use async_trait::async_trait;
use serde_json::Value;

/// Configuration validator trait
#[async_trait]
pub trait ConfigValidator: Send + Sync {
    /// Validate configuration data
    async fn validate(&self, data: &Value) -> ConfigResult<()>;

    /// Get validation schema (if applicable)
    fn schema(&self) -> Option<Value> {
        None
    }
}

/// No-op validator that always passes
#[derive(Debug, Clone)]
pub struct NoOpValidator;

#[async_trait]
impl ConfigValidator for NoOpValidator {
    async fn validate(&self, _data: &Value) -> ConfigResult<()> {
        Ok(())
    }
}

/// Schema-based validator
#[derive(Debug, Clone)]
pub struct SchemaValidator {
    /// JSON schema
    schema: Value,
}

impl SchemaValidator {
    /// Create a new schema validator
    pub fn new(schema: Value) -> Self {
        Self { schema }
    }

    /// Create from JSON schema string
    pub fn from_json(schema_json: &str) -> ConfigResult<Self> {
        let schema = serde_json::from_str(schema_json)?;
        Ok(Self::new(schema))
    }
}

#[async_trait]
impl ConfigValidator for SchemaValidator {
    async fn validate(&self, data: &Value) -> ConfigResult<()> {
        // Basic validation - in a real implementation, you'd use a JSON schema library
        self.validate_recursive(data, &self.schema, "")
    }

    fn schema(&self) -> Option<Value> {
        Some(self.schema.clone())
    }
}

impl SchemaValidator {
    /// Recursive validation helper
    fn validate_recursive(&self, data: &Value, schema: &Value, path: &str) -> ConfigResult<()> {
        match schema {
            Value::Object(schema_obj) => {
                // Check type
                if let Some(type_val) = schema_obj.get("type") {
                    if let Some(type_str) = type_val.as_str() {
                        if !self.check_type(data, type_str) {
                            return Err(ConfigError::validation_error(
                                format!("Expected type '{}' at path '{}'", type_str, path),
                                Some(path.to_string()),
                            ));
                        }
                    }
                }

                // Check required fields
                if let Some(required) = schema_obj.get("required") {
                    if let Some(required_array) = required.as_array() {
                        if let Some(data_obj) = data.as_object() {
                            for required_field in required_array {
                                if let Some(field_name) = required_field.as_str() {
                                    if !data_obj.contains_key(field_name) {
                                        return Err(ConfigError::validation_error(
                                            format!(
                                                "Required field '{}' missing at path '{}'",
                                                field_name, path
                                            ),
                                            Some(format!("{}.{}", path, field_name)),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                // Check properties
                if let Some(properties) = schema_obj.get("properties") {
                    if let Some(properties_obj) = properties.as_object() {
                        if let Some(data_obj) = data.as_object() {
                            for (prop_name, prop_schema) in properties_obj {
                                if let Some(prop_data) = data_obj.get(prop_name) {
                                    let new_path = if path.is_empty() {
                                        prop_name.clone()
                                    } else {
                                        format!("{}.{}", path, prop_name)
                                    };
                                    self.validate_recursive(prop_data, prop_schema, &new_path)?;
                                }
                            }
                        }
                    }
                }

                // Check minimum/maximum for numbers
                if data.is_number() {
                    if let Some(minimum) = schema_obj.get("minimum") {
                        if let (Some(data_num), Some(min_num)) = (data.as_f64(), minimum.as_f64()) {
                            if data_num < min_num {
                                return Err(ConfigError::validation_error(
                                    format!(
                                        "Value {} is less than minimum {} at path '{}'",
                                        data_num, min_num, path
                                    ),
                                    Some(path.to_string()),
                                ));
                            }
                        }
                    }

                    if let Some(maximum) = schema_obj.get("maximum") {
                        if let (Some(data_num), Some(max_num)) = (data.as_f64(), maximum.as_f64()) {
                            if data_num > max_num {
                                return Err(ConfigError::validation_error(
                                    format!(
                                        "Value {} is greater than maximum {} at path '{}'",
                                        data_num, max_num, path
                                    ),
                                    Some(path.to_string()),
                                ));
                            }
                        }
                    }
                }

                // Check string length
                if let Some(data_str) = data.as_str() {
                    if let Some(min_length) = schema_obj.get("minLength") {
                        if let Some(min_len) = min_length.as_u64() {
                            if (data_str.len() as u64) < min_len {
                                return Err(ConfigError::validation_error(
                                    format!(
                                        "String length {} is less than minimum {} at path '{}'",
                                        data_str.len(),
                                        min_len,
                                        path
                                    ),
                                    Some(path.to_string()),
                                ));
                            }
                        }
                    }

                    if let Some(max_length) = schema_obj.get("maxLength") {
                        if let Some(max_len) = max_length.as_u64() {
                            if (data_str.len() as u64) > max_len {
                                return Err(ConfigError::validation_error(
                                    format!(
                                        "String length {} is greater than maximum {} at path '{}'",
                                        data_str.len(),
                                        max_len,
                                        path
                                    ),
                                    Some(path.to_string()),
                                ));
                            }
                        }
                    }
                }
            }
            _ => {
                // Simple type validation
                return Err(ConfigError::validation_error(
                    format!("Invalid schema format at path '{}'", path),
                    Some(path.to_string()),
                ));
            }
        }

        Ok(())
    }

    /// Check if data matches the expected type
    fn check_type(&self, data: &Value, expected_type: &str) -> bool {
        match expected_type {
            "string" => data.is_string(),
            "number" => data.is_number(),
            "integer" => data.is_i64() || data.is_u64(),
            "boolean" => data.is_boolean(),
            "array" => data.is_array(),
            "object" => data.is_object(),
            "null" => data.is_null(),
            _ => false,
        }
    }
}

/// Function-based validator
#[derive(Debug)]
pub struct FunctionValidator<F>
where
    F: Fn(&Value) -> ConfigResult<()> + Send + Sync,
{
    /// Validation function
    validator_fn: F,
}

impl<F> FunctionValidator<F>
where
    F: Fn(&Value) -> ConfigResult<()> + Send + Sync,
{
    /// Create a new function validator
    pub fn new(validator_fn: F) -> Self {
        Self { validator_fn }
    }
}

#[async_trait]
impl<F> ConfigValidator for FunctionValidator<F>
where
    F: Fn(&Value) -> ConfigResult<()> + Send + Sync,
{
    async fn validate(&self, data: &Value) -> ConfigResult<()> {
        (self.validator_fn)(data)
    }
}

/// Composite validator that runs multiple validators
pub struct CompositeValidator {
    /// List of validators
    validators: Vec<Box<dyn ConfigValidator>>,
}

impl std::fmt::Debug for CompositeValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeValidator")
            .field(
                "validators",
                &format!("{} validators", self.validators.len()),
            )
            .finish()
    }
}

impl CompositeValidator {
    /// Create a new composite validator
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Add a validator
    pub fn add_validator(mut self, validator: Box<dyn ConfigValidator>) -> Self {
        self.validators.push(validator);
        self
    }
}

impl Default for CompositeValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConfigValidator for CompositeValidator {
    async fn validate(&self, data: &Value) -> ConfigResult<()> {
        for validator in &self.validators {
            validator.validate(data).await?;
        }
        Ok(())
    }
}
