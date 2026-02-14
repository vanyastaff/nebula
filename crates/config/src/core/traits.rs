//! Core traits for configuration system

use super::{ConfigError, ConfigResult, ConfigSource, SourceMetadata};
use async_trait::async_trait;
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
