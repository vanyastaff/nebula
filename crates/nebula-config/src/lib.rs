//! Nebula Config - configuration management for Nebula
//!
//! This crate provides a flexible and extensible configuration management system
//! with support for multiple sources, formats, validation, and hot-reloading.
//!
//! # Example
//!
//! ```rust,no_run
//! use nebula_config::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> ConfigResult<()> {
//!     // Build configuration from multiple sources
//!     let config = ConfigBuilder::new()
//!         .with_source(ConfigSource::File("config.toml".into()))
//!         .with_source(ConfigSource::Env)
//!         .with_hot_reload(true)
//!         .build()
//!         .await?;
//!
//!     // Get typed configuration
//!     let port: u16 = config.get("server.port").await?;
//!     let database_url: String = config.get("database.url").await?;
//!
//!     Ok(())
//! }
//! ```

#![deny(unused_must_use)]
// Pragmatic clippy allows for a large, complex configuration system
// These are intentional design decisions, not oversights:

// Builder pattern returns Self - hundreds of methods would need #[must_use]
#![allow(clippy::must_use_candidate)]
// Large functions in config parsing/validation are complex by nature
#![allow(clippy::too_many_lines)]
// Cast truncation in config parsing is validated elsewhere
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
// Methods returning Self in builders are intentionally not marked must_use
#![allow(clippy::return_self_not_must_use)]
// Nested blocks in parsers/validators are unavoidable for readability
#![allow(clippy::excessive_nesting)]
// Some complexity is inherent in configuration validation
#![allow(clippy::cognitive_complexity)]

// Core module with main functionality
pub mod core;

// Implementation modules
pub mod loaders;
pub mod validators;
pub mod watchers;

// Re-export main types from core for explicit imports (e.g. `use nebula_config::Config`)
pub use core::{
    Config, ConfigBuilder, ConfigError, ConfigFormat, ConfigResult, ConfigResultAggregator,
    ConfigResultExt, ConfigSource, SourceMetadata, try_sources,
};

// Re-export traits
pub use core::{
    AsyncConfigurable, ConfigLoader, ConfigValidator, ConfigWatcher, Configurable, Validatable,
};

// Re-export concrete implementations
pub use loaders::{CompositeLoader, EnvLoader, FileLoader};

pub use validators::{CompositeValidator, FunctionValidator, NoOpValidator, SchemaValidator};

pub use watchers::{
    ConfigWatchEvent, ConfigWatchEventType, FileWatcher, NoOpWatcher, PollingWatcher,
};

/// Prelude module for convenient glob imports (`use nebula_config::prelude::*`).
///
/// Includes core types, traits, and common implementations.
/// For selective imports, use the top-level re-exports instead.
pub mod prelude {

    // Core types
    pub use crate::core::{
        Config, ConfigBuilder, ConfigError, ConfigFormat, ConfigResult, ConfigResultExt,
        ConfigSource, SourceMetadata,
    };

    // Re-export nebula ecosystem types for convenience
    pub use nebula_log::{debug, error, info, warn};

    // Traits
    pub use crate::core::{
        AsyncConfigurable, ConfigLoader, ConfigValidator, ConfigWatcher, Configurable, Validatable,
    };

    // Common loaders
    pub use crate::loaders::{CompositeLoader, EnvLoader, FileLoader};

    // Common validators
    pub use crate::validators::{NoOpValidator, SchemaValidator};

    // Common watchers
    pub use crate::watchers::{
        ConfigWatchEvent, ConfigWatchEventType, FileWatcher, PollingWatcher,
    };
}

/// Builder pattern helpers for configuration
pub mod builders {
    use crate::core::{ConfigBuilder, ConfigSource};
    use crate::loaders::{EnvLoader, FileLoader};
    use crate::validators::SchemaValidator;
    use crate::watchers::FileWatcher;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Create a simple file-based configuration
    pub fn from_file(path: impl Into<PathBuf>) -> ConfigBuilder {
        ConfigBuilder::new()
            .with_source(ConfigSource::File(path.into()))
            .with_loader(Arc::new(FileLoader::new()))
    }

    /// Create a configuration from environment variables
    pub fn from_env() -> ConfigBuilder {
        ConfigBuilder::new()
            .with_source(ConfigSource::Env)
            .with_loader(Arc::new(EnvLoader::new()))
    }

    /// Create a configuration from environment with prefix
    pub fn from_env_prefix(prefix: impl Into<String>) -> ConfigBuilder {
        let prefix: String = prefix.into();
        ConfigBuilder::new()
            .with_source(ConfigSource::EnvWithPrefix(prefix.clone()))
            .with_loader(Arc::new(EnvLoader::with_prefix(prefix)))
    }

    /// Create a standard application configuration
    /// (config file + environment overrides)
    pub fn standard_app_config(config_file: impl Into<PathBuf>) -> ConfigBuilder {
        ConfigBuilder::new()
            .with_source(ConfigSource::File(config_file.into()))
            .with_source(ConfigSource::Env)
            .with_loader(Arc::new(crate::loaders::CompositeLoader::default_loaders()))
    }

    /// Create a configuration with file watching
    pub fn with_hot_reload(config_file: impl Into<PathBuf>) -> ConfigBuilder {
        ConfigBuilder::new()
            .with_source(ConfigSource::File(config_file.into()))
            .with_loader(Arc::new(FileLoader::new()))
            .with_watcher(Arc::new(FileWatcher::new(|event| {
                nebula_log::info!("Configuration changed: {:?}", event);
            })))
            .with_hot_reload(true)
    }

    /// Create a configuration with schema validation
    pub fn with_schema_validation(
        config_file: impl Into<PathBuf>,
        schema: serde_json::Value,
    ) -> ConfigBuilder {
        ConfigBuilder::new()
            .with_source(ConfigSource::File(config_file.into()))
            .with_loader(Arc::new(FileLoader::new()))
            .with_validator(Arc::new(SchemaValidator::new(schema)))
    }
}

/// Utilities for working with configuration
pub mod utils {
    use crate::core::{ConfigError, ConfigResult};
    use std::path::Path;

    /// Check if a configuration file exists and is readable
    pub async fn check_config_file(path: &Path) -> ConfigResult<()> {
        if !path.exists() {
            return Err(ConfigError::file_not_found(path));
        }

        match tokio::fs::metadata(path).await {
            Ok(metadata) if metadata.is_file() => Ok(()),
            Ok(_) => Err(ConfigError::file_read_error(path, "Path is not a file")),
            Err(e) => Err(ConfigError::file_read_error(path, e.to_string())),
        }
    }

    /// Merge multiple JSON values
    pub fn merge_json_values(values: Vec<serde_json::Value>) -> ConfigResult<serde_json::Value> {
        if values.is_empty() {
            return Ok(serde_json::Value::Object(serde_json::Map::new()));
        }

        let mut iter = values.into_iter();
        let mut result = iter
            .next()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        for value in iter {
            crate::core::config::merge_json(&mut result, value)?;
        }

        Ok(result)
    }

    /// Load configuration from a string based on format.
    /// Delegates to the shared parsers in `loaders::file`.
    pub fn parse_config_string(
        content: &str,
        format: crate::ConfigFormat,
    ) -> ConfigResult<serde_json::Value> {
        crate::loaders::file::parse_content(content, format, std::path::Path::new("string"))
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    #[test]
    fn test_prelude_imports() {
        // Ensure prelude types are accessible and constructible
        let builder = ConfigBuilder::new();
        assert!(format!("{:?}", builder).contains("ConfigBuilder"));
    }

    #[tokio::test]
    async fn test_builder_helpers() {
        use crate::builders;

        let builder = builders::from_env();
        // Builder should be created successfully
        assert!(builder.build().await.is_ok());
    }

    #[test]
    fn test_parse_config_string_yaml() {
        use crate::core::source::ConfigFormat;
        use crate::utils::parse_config_string;

        let yaml = r#"
        server:
          port: 8080
          host: "localhost"
        features:
          - a
          - b
        enabled: true
        "#;

        let value = parse_config_string(yaml, ConfigFormat::Yaml).expect("YAML should parse");
        assert_eq!(value["server"]["port"], 8080);
        assert_eq!(value["server"]["host"], "localhost");
        assert_eq!(value["enabled"], true);
    }

    #[test]
    fn test_parse_config_string_ini() {
        use crate::core::source::ConfigFormat;
        use crate::utils::parse_config_string;

        let ini = r#"
        [server]
        port=8080
        host=localhost
        enabled=true
        "#;
        let value = parse_config_string(ini, ConfigFormat::Ini).expect("INI should parse");
        assert_eq!(value["server"]["port"], 8080);
        assert_eq!(value["server"]["host"], "localhost");
        assert_eq!(value["server"]["enabled"], true);
    }

    #[test]
    fn test_parse_config_string_properties() {
        use crate::core::source::ConfigFormat;
        use crate::utils::parse_config_string;

        let properties = r#"
        server.port=8081
        server.host=localhost
        enabled=false
        "#;
        let value = parse_config_string(properties, ConfigFormat::Properties)
            .expect("Properties should parse");
        assert_eq!(value["server"]["port"], 8081);
        assert_eq!(value["server"]["host"], "localhost");
        assert_eq!(value["enabled"], false);
    }

    #[tokio::test]
    async fn test_get_with_arrays() {
        use crate::prelude::*;
        use serde_json::json;

        let defaults = json!({
            "arr": [
                {"name": "a"},
                {"name": "b"}
            ]
        });
        let config = ConfigBuilder::new()
            .with_defaults_json(defaults)
            .build()
            .await
            .expect("build ok");

        let name: String = config.get("arr.1.name").await.expect("should get string");
        assert_eq!(name, "b");

        // invalid index
        let err = config
            .get::<String>("arr.x")
            .await
            .expect_err("should error");
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid array index"));

        // out of bounds
        let err2 = config
            .get::<String>("arr.5.name")
            .await
            .expect_err("should error");
        let msg2 = format!("{}", err2);
        assert!(msg2.contains("out of bounds"));
    }
}
