//! # Nebula Configuration Management
//! 
//! Configuration management system for the Nebula workflow engine.
//! This crate provides environment-based configuration, hot-reload support,
//! validation, and defaults for all Nebula components.
//! 
//! ## Key Features
//! 
//! - **Environment-based Configuration**: Load from environment variables
//! - **File-based Configuration**: Support for JSON, TOML, and YAML
//! - **Hot-reload Support**: Watch configuration files for changes
//! - **Validation**: Schema validation and constraint checking
//! - **Defaults**: Sensible defaults for all components
//! - **Hierarchical**: Support for nested configuration structures
//! 
//! ## Usage
//! 
//! ```rust
//! use nebula_config::{Config, ConfigBuilder, ConfigSource};
//! 
//! #[derive(Debug, Clone, serde::Deserialize)]
//! struct AppConfig {
//!     database_url: String,
//!     port: u16,
//!     log_level: String,
//! }
//! 
//! let config = ConfigBuilder::new()
//!     .with_source(ConfigSource::Env)
//!     .with_source(ConfigSource::File("config.toml"))
//!     .with_defaults(AppConfig {
//!         database_url: "postgres://localhost/nebula".to_string(),
//!         port: 8080,
//!         log_level: "info".to_string(),
//!     })
//!     .build()
//!     .await?;
//! 
//! let app_config: AppConfig = config.get()?;
//! ```

pub mod builder;
pub mod error;
pub mod loader;
pub mod source;
pub mod validator;
pub mod watcher;

// Re-export main types
pub use builder::{Config, ConfigBuilder};
pub use error::ConfigError;
pub use loader::ConfigLoader;
pub use source::ConfigSource;
pub use validator::ConfigValidator;
pub use watcher::ConfigWatcher;

/// Common prelude for configuration
pub mod prelude {
    pub use super::{
        Config, ConfigBuilder, ConfigError, ConfigLoader,
        ConfigSource, ConfigValidator, ConfigWatcher,
    };
}
