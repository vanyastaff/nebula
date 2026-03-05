//! Core traits for configuration system

use super::{ConfigError, ConfigResult, ConfigSource, SourceMetadata};
use async_trait::async_trait;
use nebula_validator::foundation::{Validate, ValidationError};
use serde_json::Value;

/// Configuration loader trait
#[async_trait]
pub trait ConfigLoader: Send + Sync {
    /// Load configuration from a source
    async fn load(&self, source: &ConfigSource) -> ConfigResult<Value>;

    /// Check if the loader supports the given source
    fn supports(&self, source: &ConfigSource) -> bool;

    /// Get metadata for the source
    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata>;
}

/// Configuration validator trait
#[async_trait]
pub trait ConfigValidator: Send + Sync {
    /// Validate configuration data
    async fn validate(&self, data: &Value) -> ConfigResult<()>;

    /// Get validation schema if available
    fn schema(&self) -> Option<Value> {
        None
    }

    /// Get validation rules description
    fn rules(&self) -> Option<String> {
        None
    }
}

fn map_validation_error(err: ValidationError) -> ConfigError {
    let mut message = if err.code.is_empty() {
        err.message.to_string()
    } else {
        format!("[{}] {}", err.code, err.message)
    };

    if let Some(help) = err.help() {
        message.push_str("; help: ");
        message.push_str(help);
    }

    let total_errors = err.total_error_count();
    if total_errors > 1 {
        message.push_str("; nested_errors=");
        message.push_str(&(total_errors - 1).to_string());
    }

    let field = err.field_pointer().map(std::borrow::Cow::into_owned).or_else(|| {
        err.flatten()
            .into_iter()
            .skip(1)
            .find_map(|nested| nested.field_pointer().map(std::borrow::Cow::into_owned))
    });

    ConfigError::validation_error(
        message,
        field,
    )
}

#[async_trait]
impl<T> ConfigValidator for T
where
    T: Validate<Value> + Send + Sync,
{
    async fn validate(&self, data: &Value) -> ConfigResult<()> {
        Validate::validate(self, data).map_err(map_validation_error)
    }
}

/// Configuration watcher trait
#[async_trait]
pub trait ConfigWatcher: Send + Sync {
    /// Start watching configuration sources
    async fn start_watching(&self, sources: &[ConfigSource]) -> ConfigResult<()>;

    /// Stop watching
    async fn stop_watching(&self) -> ConfigResult<()>;

    /// Check if currently watching
    fn is_watching(&self) -> bool;
}

/// Trait for configuration validation and management
pub trait Validatable: Send + Sync {
    /// Validate configuration
    fn validate(&self) -> Result<(), ConfigError>;

    /// Get default configuration
    fn default_config() -> Self
    where
        Self: Sized;

    /// Merge with another configuration
    fn merge(&mut self, other: Self)
    where
        Self: Sized;

    /// Check if configuration is valid
    fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

/// Configurable trait for components
pub trait Configurable: Send + Sync {
    /// Configuration type
    type Config: Validatable;

    /// Apply configuration
    fn configure(&mut self, config: Self::Config) -> Result<(), ConfigError>;

    /// Get current configuration
    fn configuration(&self) -> &Self::Config;

    /// Reset to default configuration
    fn reset_config(&mut self) -> Result<(), ConfigError> {
        self.configure(Self::Config::default_config())
    }
}

/// Async configurable trait
#[async_trait]
pub trait AsyncConfigurable: Send + Sync {
    /// Configuration type
    type Config: Validatable;

    /// Apply configuration asynchronously
    async fn configure(&mut self, config: Self::Config) -> Result<(), ConfigError>;

    /// Get current configuration
    fn configuration(&self) -> &Self::Config;

    /// Reset to default configuration
    async fn reset_config(&mut self) -> Result<(), ConfigError> {
        self.configure(Self::Config::default_config()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct NestedFieldValidator;

    impl Validate<Value> for NestedFieldValidator {
        fn validate(&self, _input: &Value) -> Result<(), ValidationError> {
            Err(
                ValidationError::new("validation_failed", "top level failed")
                    .with_help("set service.port to a valid value")
                    .with_nested(vec![
                        ValidationError::new("type_mismatch", "port must be integer")
                            .with_field("service.port"),
                    ]),
            )
        }
    }

    #[tokio::test]
    async fn bridge_preserves_code_and_nested_field_context() {
        let err = ConfigValidator::validate(&NestedFieldValidator, &serde_json::json!({}))
            .await
            .expect_err("validator should fail");

        match err {
            ConfigError::ValidationError { message, field } => {
                assert!(message.contains("[validation_failed]"));
                assert!(message.contains("help: set service.port to a valid value"));
                assert!(message.contains("nested_errors=1"));
                assert_eq!(field.as_deref(), Some("/service/port"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }
}
