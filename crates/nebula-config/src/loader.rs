//! Configuration loader implementation

use crate::{ConfigError, ConfigResult, ConfigSource, ConfigFormat, SourceMetadata};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::path::Path;

/// Configuration loader trait
#[async_trait]
pub trait ConfigLoader: Send + Sync {
    /// Load configuration from a source
    async fn load(&self, source: &ConfigSource) -> ConfigResult<serde_json::Value>;
    
    /// Check if the loader supports the given source
    fn supports(&self, source: &ConfigSource) -> bool;
    
    /// Get metadata for the source
    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata>;
}

/// File-based configuration loader
#[derive(Debug, Clone)]
pub struct FileLoader {
    /// Base directory for relative paths
    pub base_dir: Option<std::path::PathBuf>,
}

impl FileLoader {
    /// Create a new file loader
    pub fn new() -> Self {
        Self { base_dir: None }
    }
    
    /// Create a new file loader with base directory
    pub fn with_base_dir(base_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            base_dir: Some(base_dir.into()),
        }
    }
    
    /// Resolve path relative to base directory
    fn resolve_path(&self, path: &Path) -> std::path::PathBuf {
        if let Some(base_dir) = &self.base_dir {
            if path.is_relative() {
                base_dir.join(path)
            } else {
                path.to_path_buf()
            }
        } else {
            path.to_path_buf()
        }
    }
    
    /// Parse configuration content based on format
    fn parse_content(&self, content: &str, format: ConfigFormat) -> ConfigResult<serde_json::Value> {
        match format {
            ConfigFormat::Json => {
                serde_json::from_str(content).map_err(ConfigError::from)
            }
            ConfigFormat::Toml => {
                let value: toml::Value = toml::from_str(content)?;
                Ok(serde_json::to_value(value)?)
            }
            ConfigFormat::Yaml => {
                let docs = yaml_rust::YamlLoader::load_from_str(content)?;
                if docs.is_empty() {
                    Ok(serde_json::Value::Null)
                } else {
                    // Convert YAML to JSON value
                    let yaml_value = &docs[0];
                    self.yaml_to_json(yaml_value)
                }
            }
            _ => Err(ConfigError::format_not_supported(format.to_string())),
        }
    }
    
    /// Convert YAML value to JSON value
    fn yaml_to_json(&self, yaml: &yaml_rust::Yaml) -> ConfigResult<serde_json::Value> {
        match yaml {
            yaml_rust::Yaml::Real(s) | yaml_rust::Yaml::String(s) => {
                Ok(serde_json::Value::String(s.clone()))
            }
            yaml_rust::Yaml::Integer(i) => {
                Ok(serde_json::Value::Number(serde_json::Number::from(*i)))
            }
            yaml_rust::Yaml::Boolean(b) => {
                Ok(serde_json::Value::Bool(*b))
            }
            yaml_rust::Yaml::Array(arr) => {
                let mut json_arr = Vec::new();
                for item in arr {
                    json_arr.push(self.yaml_to_json(item)?);
                }
                Ok(serde_json::Value::Array(json_arr))
            }
            yaml_rust::Yaml::Hash(hash) => {
                let mut json_obj = serde_json::Map::new();
                for (key, value) in hash {
                    let key_str = match key {
                        yaml_rust::Yaml::String(s) => s.clone(),
                        yaml_rust::Yaml::Integer(i) => i.to_string(),
                        _ => return Err(ConfigError::parse_error(
                            std::path::PathBuf::from("yaml"),
                            "Invalid key type in YAML hash"
                        )),
                    };
                    json_obj.insert(key_str, self.yaml_to_json(value)?);
                }
                Ok(serde_json::Value::Object(json_obj))
            }
            yaml_rust::Yaml::Null => Ok(serde_json::Value::Null),
            _ => Err(ConfigError::parse_error(
                std::path::PathBuf::from("yaml"),
                "Unsupported YAML type"
            )),
        }
    }
}

impl Default for FileLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConfigLoader for FileLoader {
    async fn load(&self, source: &ConfigSource) -> ConfigResult<serde_json::Value> {
        match source {
            ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                let resolved_path = self.resolve_path(path);

                if !resolved_path.exists() {
                    return Err(ConfigError::file_not_found(&resolved_path));
                }

                let content = tokio::fs::read_to_string(&resolved_path)
                    .await
                    .map_err(|e| ConfigError::file_read_error(&resolved_path, e.to_string()))?;

                let format = ConfigFormat::from_path(&resolved_path);
                self.parse_content(&content, format)
            }
            _ => Err(ConfigError::source_error(
                "FileLoader does not support this source type",
                source.name()
            )),
        }
    }

    fn supports(&self, source: &ConfigSource) -> bool {
        source.is_file_based()
    }

    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata> {
        match source {
            ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                let resolved_path = self.resolve_path(path);

                if !resolved_path.exists() {
                    return Err(ConfigError::file_not_found(&resolved_path));
                }

                let metadata = tokio::fs::metadata(&resolved_path)
                    .await
                    .map_err(|e| ConfigError::file_read_error(&resolved_path, e.to_string()))?;

                let format = ConfigFormat::from_path(&resolved_path);

                Ok(SourceMetadata::new(source.clone())
                    .with_size(metadata.len())
                    .with_format(format)
                    .with_last_modified(
                        metadata.modified()
                            .ok()
                            .and_then(|t| chrono::DateTime::from_timestamp(
                                t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64, 0
                            ))
                            .unwrap_or_else(chrono::Utc::now)
                    ))
            }
            _ => Err(ConfigError::source_error(
                "FileLoader does not support this source type",
                source.name()
            )),
        }
    }
}

/// Environment variable loader
#[derive(Debug, Clone)]
pub struct EnvLoader {
    /// Environment variable prefix
    pub prefix: Option<String>,

    /// Separator for nested keys
    pub separator: String,

    /// Case sensitivity
    pub case_sensitive: bool,
}

impl EnvLoader {
    /// Create a new environment loader
    pub fn new() -> Self {
        Self {
            prefix: None,
            separator: "_".to_string(),
            case_sensitive: false,
        }
    }

    /// Create a new environment loader with prefix
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: Some(prefix.into()),
            separator: "_".to_string(),
            case_sensitive: false,
        }
    }

    /// Set separator for nested keys
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }

    /// Set case sensitivity
    pub fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    /// Convert environment variables to nested JSON structure
    fn env_to_json(&self, vars: HashMap<String, String>) -> serde_json::Value {
        let mut result = serde_json::Map::new();

        for (key, value) in vars {
            let parts: Vec<&str> = key.split(&self.separator).collect();
            self.insert_nested(&mut result, &parts, value);
        }

        serde_json::Value::Object(result)
    }

    /// Insert value into nested structure
    fn insert_nested(&self, obj: &mut serde_json::Map<String, serde_json::Value>, parts: &[&str], value: String) {
        if parts.is_empty() {
            return;
        }

        if parts.len() == 1 {
            let parsed_value = self.parse_env_value(&value);
            obj.insert(parts[0].to_string(), parsed_value);
            return;
        }

        let key = parts[0].to_string();
        let remaining = &parts[1..];

        let nested = obj.entry(key).or_insert_with(|| {
            serde_json::Value::Object(serde_json::Map::new())
        });

        if let serde_json::Value::Object(nested_obj) = nested {
            self.insert_nested(nested_obj, remaining, value);
        }
    }

    /// Parse environment variable value
    fn parse_env_value(&self, value: &str) -> serde_json::Value {
        // Try to parse as different types
        if let Ok(bool_val) = value.parse::<bool>() {
            return serde_json::Value::Bool(bool_val);
        }

        if let Ok(int_val) = value.parse::<i64>() {
            return serde_json::Value::Number(serde_json::Number::from(int_val));
        }

        if let Ok(float_val) = value.parse::<f64>() {
            if let Some(num) = serde_json::Number::from_f64(float_val) {
                return serde_json::Value::Number(num);
            }
        }

        // Try to parse as JSON
        if value.starts_with('{') || value.starts_with('[') {
            if let Ok(json_val) = serde_json::from_str(value) {
                return json_val;
            }
        }

        // Default to string
        serde_json::Value::String(value.to_string())
    }
}

impl Default for EnvLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConfigLoader for EnvLoader {
    async fn load(&self, source: &ConfigSource) -> ConfigResult<serde_json::Value> {
        match source {
            ConfigSource::Env => {
                let vars: HashMap<String, String> = std::env::vars().collect();
                Ok(self.env_to_json(vars))
            }
            ConfigSource::EnvWithPrefix(prefix) => {
                let vars: HashMap<String, String> = std::env::vars()
                    .filter_map(|(key, value)| {
                        let key_to_check = if self.case_sensitive {
                            key.clone()
                        } else {
                            key.to_uppercase()
                        };

                        let prefix_to_check = if self.case_sensitive {
                            prefix.clone()
                        } else {
                            prefix.to_uppercase()
                        };

                        if key_to_check.starts_with(&prefix_to_check) {
                            let stripped_key = key_to_check.strip_prefix(&prefix_to_check)
                                .unwrap_or(&key_to_check)
                                .trim_start_matches(&self.separator);
                            Some((stripped_key.to_string(), value))
                        } else {
                            None
                        }
                    })
                    .collect();

                Ok(self.env_to_json(vars))
            }
            _ => Err(ConfigError::source_error(
                "EnvLoader does not support this source type",
                source.name()
            )),
        }
    }

    fn supports(&self, source: &ConfigSource) -> bool {
        source.is_env_based()
    }

    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata> {
        match source {
            ConfigSource::Env | ConfigSource::EnvWithPrefix(_) => {
                Ok(SourceMetadata::new(source.clone())
                    .with_format(ConfigFormat::Env)
                    .with_last_modified(chrono::Utc::now()))
            }
            _ => Err(ConfigError::source_error(
                "EnvLoader does not support this source type",
                source.name()
            )),
        }
    }
}

/// Composite configuration loader
pub struct CompositeLoader {
    /// List of loaders
    loaders: Vec<Box<dyn ConfigLoader>>,
}

impl std::fmt::Debug for CompositeLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeLoader")
            .field("loaders", &format!("{} loaders", self.loaders.len()))
            .finish()
    }
}

impl CompositeLoader {
    /// Create a new composite loader
    pub fn new() -> Self {
        Self {
            loaders: Vec::new(),
        }
    }

    /// Add a loader
    pub fn add_loader(mut self, loader: Box<dyn ConfigLoader>) -> Self {
        self.loaders.push(loader);
        self
    }

    /// Create default composite loader with file and env loaders
    pub fn default_loaders() -> Self {
        Self::new()
            .add_loader(Box::new(FileLoader::new()))
            .add_loader(Box::new(EnvLoader::new()))
    }
}

impl Default for CompositeLoader {
    fn default() -> Self {
        Self::default_loaders()
    }
}

#[async_trait]
impl ConfigLoader for CompositeLoader {
    async fn load(&self, source: &ConfigSource) -> ConfigResult<serde_json::Value> {
        for loader in &self.loaders {
            if loader.supports(source) {
                return loader.load(source).await;
            }
        }
        
        Err(ConfigError::source_error(
            "No loader supports this source type",
            source.name()
        ))
    }
    
    fn supports(&self, source: &ConfigSource) -> bool {
        self.loaders.iter().any(|loader| loader.supports(source))
    }
    
    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata> {
        for loader in &self.loaders {
            if loader.supports(source) {
                return loader.metadata(source).await;
            }
        }
        
        Err(ConfigError::source_error(
            "No loader supports this source type",
            source.name()
        ))
    }
}
