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
//!     let port: u16 = config.get_path("server.port").await?;
//!     let database_url: String = config.get_path("database.url").await?;
//!
//!     Ok(())
//! }
//! ```

#![deny(unused_must_use)]
#![warn(missing_docs)]

// Core module with main functionality
pub mod core;

// Implementation modules
pub mod loaders;
pub mod validators;
pub mod watchers;

// Re-export main types from core
pub use core::{
    Config,
    ConfigBuilder,
    ConfigError,
    ConfigResult,
    ConfigSource,
    ConfigFormat,
    SourceMetadata,
    ConfigResultExt,
    ConfigResultAggregator,
    try_sources,
};

// Re-export traits
pub use core::{
    ConfigLoader,
    ConfigValidator,
    ConfigWatcher,
    Validatable,
    Configurable,
    AsyncConfigurable,
};

// Re-export concrete implementations
pub use loaders::{
    CompositeLoader,
    EnvLoader,
    FileLoader,
};

pub use validators::{
    CompositeValidator,
    NoOpValidator,
    SchemaValidator,
    FunctionValidator,
};

pub use watchers::{
    FileWatcher,
    PollingWatcher,
    NoOpWatcher,
    ConfigWatchEvent,
    ConfigWatchEventType,
};

/// Prelude module for convenient imports
pub mod prelude {
    //! Prelude for common imports
    //!
    //! # Example
    //! ```rust
    //! use nebula_config::prelude::*;
    //! ```

    // Core types
    pub use crate::core::{
        Config,
        ConfigBuilder,
        ConfigError,
        ConfigResult,
        ConfigSource,
        ConfigFormat,
        SourceMetadata,
        ConfigResultExt,
    };

    // Re-export nebula ecosystem types for convenience
    pub use nebula_value::Value as NebulaValue;
    pub use nebula_error::NebulaError;
    pub use nebula_log::{debug, info, warn, error};

    // Traits
    pub use crate::core::{
        ConfigLoader,
        ConfigValidator,
        ConfigWatcher,
        Validatable,
        Configurable,
        AsyncConfigurable,
    };

    // Common loaders
    pub use crate::loaders::{
        CompositeLoader,
        EnvLoader,
        FileLoader,
    };

    // Common validators
    pub use crate::validators::{
        NoOpValidator,
        SchemaValidator,
    };

    // Common watchers
    pub use crate::watchers::{
        FileWatcher,
        PollingWatcher,
        ConfigWatchEvent,
        ConfigWatchEventType,
    };
}

/// Builder pattern helpers
pub mod builders {
    //! Builder utilities for configuration

    use crate::core::{ConfigBuilder, ConfigSource};
    use crate::loaders::{FileLoader, EnvLoader};
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
    //! Utility functions for configuration management

    use crate::core::{ConfigResult, ConfigError};
    use std::path::Path;

    /// Check if a configuration file exists and is readable
    pub async fn check_config_file(path: &Path) -> ConfigResult<()> {
        if !path.exists() {
            return Err(ConfigError::file_not_found(path));
        }

        match tokio::fs::metadata(path).await {
            Ok(metadata) if metadata.is_file() => Ok(()),
            Ok(_) => Err(ConfigError::file_read_error(
                path,
                "Path is not a file"
            )),
            Err(e) => Err(ConfigError::file_read_error(path, e.to_string())),
        }
    }

    /// Merge multiple JSON values
    pub fn merge_json_values(
        values: Vec<serde_json::Value>,
    ) -> ConfigResult<serde_json::Value> {
        if values.is_empty() {
            return Ok(serde_json::Value::Object(serde_json::Map::new()));
        }

        let mut iter = values.into_iter();
        let mut result = if let Some(v) = iter.next() {
            v
        } else {
            // Defensive fallback: should be unreachable due to the is_empty() guard above
            serde_json::Value::Object(serde_json::Map::new())
        };
        let temp_config = crate::Config::new(
            serde_json::Value::Object(serde_json::Map::new()),
            vec![],
            std::sync::Arc::new(crate::loaders::CompositeLoader::default()),
            None,
            None,
            false,
        );

        for value in iter {
            temp_config.merge_values(&mut result, value)?;
        }

        Ok(result)
    }

    /// Load configuration from a string based on format
    pub fn parse_config_string(
        content: &str,
        format: crate::ConfigFormat,
    ) -> ConfigResult<serde_json::Value> {
        match format {
            crate::ConfigFormat::Json => {
                serde_json::from_str(content).map_err(Into::into)
            }
            crate::ConfigFormat::Toml => {
                toml::from_str::<toml::Value>(content)?
                    .try_into()
                    .map_err(|e| ConfigError::parse_error(
                        std::path::PathBuf::from("string"),
                        format!("TOML conversion error: {}", e)
                    ))
            }
            crate::ConfigFormat::Yaml => {
                // Parse YAML using yaml_rust and convert to JSON
                use yaml_rust::YamlLoader;
                let docs = YamlLoader::load_from_str(content)
                    .map_err(|e| ConfigError::parse_error(
                        std::path::PathBuf::from("string"),
                        format!("YAML parse error: {:?}", e)
                    ))?;
                if docs.is_empty() {
                    return Ok(serde_json::Value::Null);
                }
                fn yaml_to_json(yaml: &yaml_rust::Yaml) -> ConfigResult<serde_json::Value> {
                    use yaml_rust::Yaml;
                    Ok(match yaml {
                        Yaml::Real(s) | Yaml::String(s) => {
                            if let Ok(num) = s.parse::<f64>() {
                                if let Some(json_num) = serde_json::Number::from_f64(num) {
                                    serde_json::Value::Number(json_num)
                                } else {
                                    serde_json::Value::String(s.clone())
                                }
                            } else {
                                serde_json::Value::String(s.clone())
                            }
                        }
                        Yaml::Integer(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
                        Yaml::Boolean(b) => serde_json::Value::Bool(*b),
                        Yaml::Array(arr) => {
                            let mut json_arr = Vec::new();
                            for item in arr {
                                json_arr.push(yaml_to_json(item)?);
                            }
                            serde_json::Value::Array(json_arr)
                        }
                        Yaml::Hash(hash) => {
                            let mut json_obj = serde_json::Map::new();
                            for (key, value) in hash {
                                let key_str = match key {
                                    Yaml::String(s) => s.clone(),
                                    Yaml::Integer(i) => i.to_string(),
                                    _ => {
                                        return Err(ConfigError::parse_error(
                                            std::path::PathBuf::from("string"),
                                            "Invalid key type in YAML hash",
                                        ));
                                    }
                                };
                                json_obj.insert(key_str, yaml_to_json(value)?);
                            }
                            serde_json::Value::Object(json_obj)
                        }
                        Yaml::Null => serde_json::Value::Null,
                        Yaml::BadValue => {
                            return Err(ConfigError::parse_error(
                                std::path::PathBuf::from("string"),
                                "Bad YAML value encountered",
                            ));
                        }
                        _ => {
                            return Err(ConfigError::parse_error(
                                std::path::PathBuf::from("string"),
                                "Unsupported YAML type",
                            ));
                        }
                    })
                }
                yaml_to_json(&docs[0])
            }
            crate::ConfigFormat::Ini => {
                // Inline INI parser mirroring FileLoader behavior
                let mut result = serde_json::Map::new();
                let mut current_section: Option<String> = None;
                for (line_num, line) in content.lines().enumerate() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                        continue;
                    }
                    if line.starts_with('[') && line.ends_with(']') {
                        current_section = Some(line[1..line.len()-1].to_string());
                        if let Some(section) = &current_section {
                            result.entry(section.clone())
                                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                        }
                        continue;
                    }
                    if let Some(eq_pos) = line.find('=') {
                        let key = line[..eq_pos].trim();
                        let mut value = line[eq_pos + 1..].trim();
                        // Remove quotes if present
                        if (value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\'')) {
                            value = &value[1..value.len()-1];
                        }
                        // Parse value similar to FileLoader::parse_ini_value
                        let parsed_value = if value.eq_ignore_ascii_case("true") {
                            serde_json::Value::Bool(true)
                        } else if value.eq_ignore_ascii_case("false") {
                            serde_json::Value::Bool(false)
                        } else if let Ok(int_val) = value.parse::<i64>() {
                            serde_json::Value::Number(serde_json::Number::from(int_val))
                        } else if let Ok(float_val) = value.parse::<f64>() {
                            if let Some(num) = serde_json::Number::from_f64(float_val) {
                                serde_json::Value::Number(num)
                            } else {
                                serde_json::Value::String(value.to_string())
                            }
                        } else {
                            serde_json::Value::String(value.to_string())
                        };
                        if let Some(ref section) = current_section {
                            if let Some(serde_json::Value::Object(section_obj)) = result.get_mut(section) {
                                section_obj.insert(key.to_string(), parsed_value);
                            }
                        } else {
                            result.insert(key.to_string(), parsed_value);
                        }
                    } else {
                        return Err(ConfigError::parse_error(
                            std::path::PathBuf::from("string"),
                            format!("Invalid INI format at line {}", line_num + 1)
                        ));
                    }
                }
                Ok(serde_json::Value::Object(result))
            }
            crate::ConfigFormat::Properties => {
                let mut result = serde_json::Map::new();
                // Helpers to insert dot-notation keys
                fn parse_value(v: &str) -> serde_json::Value {
                    // Reuse same parsing rules as INI values
                    if v.eq_ignore_ascii_case("true") {
                        return serde_json::Value::Bool(true);
                    }
                    if v.eq_ignore_ascii_case("false") {
                        return serde_json::Value::Bool(false);
                    }
                    if let Ok(int_val) = v.parse::<i64>() {
                        return serde_json::Value::Number(serde_json::Number::from(int_val));
                    }
                    if let Ok(float_val) = v.parse::<f64>() {
                        if let Some(num) = serde_json::Number::from_f64(float_val) {
                            return serde_json::Value::Number(num);
                        }
                    }
                    serde_json::Value::String(v.to_string())
                }
                fn insert_property(
                    obj: &mut serde_json::Map<String, serde_json::Value>,
                    key: &str,
                    value: &str,
                ) {
                    let parts: Vec<&str> = key.split('.').collect();
                    fn insert_recursive(
                        obj: &mut serde_json::Map<String, serde_json::Value>,
                        parts: &[&str],
                        value: &str,
                    ) {
                        if parts.is_empty() { return; }
                        if parts.len() == 1 {
                            obj.insert(parts[0].to_string(), parse_value(value));
                            return;
                        }
                        let entry = obj
                            .entry(parts[0].to_string())
                            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                        if let serde_json::Value::Object(map) = entry {
                            insert_recursive(map, &parts[1..], value);
                        } else {
                            *entry = serde_json::Value::Object(serde_json::Map::new());
                            if let serde_json::Value::Object(map) = entry {
                                insert_recursive(map, &parts[1..], value);
                            }
                        }
                    }
                    insert_recursive(obj, &parts, value);
                }
                for (line_num, line) in content.lines().enumerate() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                        continue;
                    }
                    let separator_pos = line.find('=') .or_else(|| line.find(':'));
                    if let Some(pos) = separator_pos {
                        let key = line[..pos].trim();
                        let value = line[pos + 1..].trim();
                        insert_property(&mut result, key, value);
                    } else {
                        return Err(ConfigError::parse_error(
                            std::path::PathBuf::from("string"),
                            format!("Invalid properties format at line {}", line_num + 1)
                        ));
                    }
                }
                Ok(serde_json::Value::Object(result))
            }
            _ => Err(ConfigError::format_not_supported(format.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    #[test]
    fn test_prelude_imports() {
        // This test just ensures prelude can be imported
        let _builder = ConfigBuilder::new();
        assert!(true);
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
        use crate::utils::parse_config_string;
        use crate::core::source::ConfigFormat;

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
        use crate::utils::parse_config_string;
        use crate::core::source::ConfigFormat;

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
        use crate::utils::parse_config_string;
        use crate::core::source::ConfigFormat;

        let properties = r#"
        server.port=8081
        server.host=localhost
        enabled=false
        "#;
        let value = parse_config_string(properties, ConfigFormat::Properties).expect("Properties should parse");
        assert_eq!(value["server"]["port"], 8081);
        assert_eq!(value["server"]["host"], "localhost");
        assert_eq!(value["enabled"], false);
    }

    #[tokio::test]
    async fn test_get_path_with_arrays() {
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

        let name: String = config.get_path("arr.1.name").await.expect("should get string");
        assert_eq!(name, "b");

        // invalid index
        let err = config.get_path::<String>("arr.x").await.expect_err("should error");
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid array index"));

        // out of bounds
        let err2 = config.get_path::<String>("arr.5.name").await.expect_err("should error");
        let msg2 = format!("{}", err2);
        assert!(msg2.contains("out of bounds"));
    }
}