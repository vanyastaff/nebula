//! Configuration types and validation using nebula-config

use std::time::Duration;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// Re-export nebula-config types for convenience
pub use nebula_config::{
    Config as NebulaConfig,
    ConfigBuilder,
    ConfigSource,
    ConfigResult,
    ConfigError,
    prelude::*,
};

/// Base configuration trait for resilience patterns
pub trait ResilienceConfig: Send + Sync + Serialize + for<'de> Deserialize<'de> + Clone {
    /// Validate configuration using nebula-config
    fn validate(&self) -> ConfigResult<()>;

    /// Get default configuration
    fn default_config() -> Self where Self: Sized;

    /// Merge with another configuration
    fn merge(&mut self, other: Self) where Self: Sized;

    /// Convert to nebula-value for dynamic configuration
    fn to_value(&self) -> nebula_value::Value {
        nebula_value::to_value(self).unwrap_or_default()
    }

    /// Create from nebula-value
    fn from_value(value: &nebula_value::Value) -> ConfigResult<Self> where Self: Sized {
        nebula_value::from_value(value)
            .map_err(|e| ConfigError::validation(format!("Failed to deserialize config: {}", e)))
    }
}

/// Common configuration parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonConfig {
    /// Operation timeout
    #[serde(with = "humantime_serde")]
    pub timeout: Option<Duration>,

    /// Enable metrics collection
    pub metrics_enabled: bool,

    /// Enable debug logging
    pub debug_enabled: bool,

    /// Service name
    pub service_name: String,

    /// Environment
    pub environment: Environment,
}

impl Default for CommonConfig {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            metrics_enabled: true,
            debug_enabled: false,
            service_name: "default".to_string(),
            environment: Environment::Production,
        }
    }
}

impl ResilienceConfig for CommonConfig {
    fn validate(&self) -> ConfigResult<()> {
        if let Some(timeout) = self.timeout {
            if timeout.as_millis() == 0 {
                return Err(ConfigError::validation("Timeout must be greater than 0"));
            }
        }

        if self.service_name.is_empty() {
            return Err(ConfigError::validation("Service name cannot be empty"));
        }

        Ok(())
    }

    fn default_config() -> Self {
        Self::default()
    }

    fn merge(&mut self, other: Self) {
        if other.timeout.is_some() {
            self.timeout = other.timeout;
        }
        self.metrics_enabled = other.metrics_enabled;
        self.debug_enabled = other.debug_enabled;
        if !other.service_name.is_empty() {
            self.service_name = other.service_name;
        }
        self.environment = other.environment;
    }
}

/// Environment enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl Default for Environment {
    fn default() -> Self {
        Self::Production
    }
}

/// Configurable trait for resilience patterns
pub trait Configurable {
    type Config: ResilienceConfig;

    /// Apply configuration
    fn configure(&mut self, config: Self::Config) -> ConfigResult<()>;

    /// Get current configuration
    fn configuration(&self) -> &Self::Config;
}

/// Configuration manager for resilience patterns
pub struct ResilienceConfigManager {
    config: NebulaConfig,
}

impl ResilienceConfigManager {
    /// Create a new configuration manager
    pub async fn new() -> ConfigResult<Self> {
        let config = ConfigBuilder::new()
            .with_source(ConfigSource::Env)
            .build()
            .await?;

        Ok(Self { config })
    }

    /// Create from existing nebula config
    pub fn from_config(config: NebulaConfig) -> Self {
        Self { config }
    }

    /// Get configuration for a specific pattern
    pub async fn get_pattern_config<T: ResilienceConfig>(&self, path: &str) -> ConfigResult<T> {
        let value = self.config.get_path(path).await?;
        T::from_value(&value)
    }

    /// Update configuration for a pattern
    pub async fn update_pattern_config<T: ResilienceConfig>(&mut self, path: &str, config: &T) -> ConfigResult<()> {
        let value = config.to_value();
        self.config.set_path(path, value).await
    }
}

/// Macro for resilience configuration validation
#[macro_export]
macro_rules! validate_resilience_config {
    ($config:expr, $($field:ident : $validator:expr),* $(,)?) => {{
        $(
            if let Err(e) = $validator(&$config.$field) {
                return Err(nebula_config::ConfigError::validation(
                    format!("Field '{}': {}", stringify!($field), e)
                ));
            }
        )*
        Ok(())
    }};
}