//! File-based configuration loader

// Standard library
use std::path::{Path, PathBuf};

// External dependencies
use async_trait::async_trait;

// Internal crates
use crate::core::{
    ConfigError, ConfigFormat, ConfigLoader, ConfigResult, ConfigSource, SourceMetadata,
};

/// File-based configuration loader
#[derive(Debug, Clone)]
pub struct FileLoader {
    /// Base directory for relative paths
    pub base_dir: Option<PathBuf>,
    /// Whether to allow missing files
    pub allow_missing: bool,
}

impl FileLoader {
    /// Create a new file loader
    pub fn new() -> Self {
        Self {
            base_dir: None,
            allow_missing: false,
        }
    }

    /// Create a new file loader with base directory
    pub fn with_base_dir(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: Some(base_dir.into()),
            allow_missing: false,
        }
    }

    /// Set whether to allow missing files
    #[must_use = "builder methods must be chained or built"]
    pub fn allow_missing(mut self, allow: bool) -> Self {
        self.allow_missing = allow;
        self
    }

    /// Resolve path relative to base directory
    fn resolve_path(&self, path: &Path) -> PathBuf {
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
}

// ==================== Standalone parsing functions ====================
// These are used by both FileLoader and utils::parse_config_string.

/// Parse configuration content based on format
pub(crate) fn parse_content(
    content: &str,
    format: ConfigFormat,
    path: &Path,
) -> ConfigResult<serde_json::Value> {
    match format {
        ConfigFormat::Toml => {
            #[cfg(feature = "toml")]
            {
                toml::from_str::<serde_json::Value>(content)
                    .map_err(|e| ConfigError::parse_error(path, format!("TOML parse error: {}", e)))
            }
            #[cfg(not(feature = "toml"))]
            {
                let _ = content;
                Err(ConfigError::format_not_supported("toml"))
            }
        },
        ConfigFormat::Yaml => {
            #[cfg(feature = "yaml")]
            {
                nebula_log::debug!("parsing config file as YAML: {}", path.display());
                serde_yaml::from_str::<serde_json::Value>(content)
                    .map_err(|e| ConfigError::parse_error(path, format!("YAML parse error: {}", e)))
            }
            #[cfg(not(feature = "yaml"))]
            {
                let _ = content;
                Err(ConfigError::format_not_supported("yaml"))
            }
        },
        ConfigFormat::Json => serde_json::from_str::<serde_json::Value>(content)
            .map_err(|e| ConfigError::parse_error(path, format!("JSON parse error: {e}"))),
        _ => Err(ConfigError::format_not_supported(format.to_string())),
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
                    if self.allow_missing {
                        nebula_log::debug!(
                            "Configuration file not found, using empty config: {}",
                            resolved_path.display()
                        );
                        return Ok(serde_json::Value::Object(serde_json::Map::new()));
                    }
                    return Err(ConfigError::file_not_found(&resolved_path));
                }

                let content = tokio::fs::read_to_string(&resolved_path)
                    .await
                    .map_err(|e| ConfigError::file_read_error(&resolved_path, e.to_string()))?;

                let format = ConfigFormat::from_path(&resolved_path);
                parse_content(&content, format, &resolved_path)
            },
            ConfigSource::Directory(dir_path) => self.load_directory(dir_path).await,
            _ => Err(ConfigError::source_error(
                "FileLoader does not support this source type",
                source.name(),
            )),
        }
    }

    fn supports(&self, source: &ConfigSource) -> bool {
        matches!(
            source,
            ConfigSource::File(_) | ConfigSource::FileAuto(_) | ConfigSource::Directory(_)
        )
    }

    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata> {
        match source {
            ConfigSource::File(path) | ConfigSource::FileAuto(path) => {
                let resolved_path = self.resolve_path(path);

                if !resolved_path.exists() {
                    if self.allow_missing {
                        return Ok(SourceMetadata::new(source.clone())
                            .with_format(ConfigFormat::from_path(&resolved_path))
                            .with_last_modified(chrono::Utc::now()));
                    }
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
                        metadata
                            .modified()
                            .ok()
                            .and_then(|t| {
                                chrono::DateTime::from_timestamp(
                                    t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64,
                                    0,
                                )
                            })
                            .unwrap_or_else(chrono::Utc::now),
                    ))
            },
            ConfigSource::Directory(_path) => Ok(SourceMetadata::new(source.clone())
                .with_format(ConfigFormat::Unknown("directory".to_string()))
                .with_last_modified(chrono::Utc::now())),
            _ => Err(ConfigError::source_error(
                "FileLoader does not support this source type",
                source.name(),
            )),
        }
    }
}

impl FileLoader {
    /// Load all config files from a directory
    async fn load_directory(&self, dir_path: &Path) -> ConfigResult<serde_json::Value> {
        let resolved_path = self.resolve_path(dir_path);

        if !resolved_path.exists() {
            if self.allow_missing {
                return Ok(serde_json::Value::Object(serde_json::Map::new()));
            }
            return Err(ConfigError::file_not_found(&resolved_path));
        }

        let mut result = serde_json::Map::new();
        let mut entries = tokio::fs::read_dir(&resolved_path)
            .await
            .map_err(|e| ConfigError::file_read_error(&resolved_path, e.to_string()))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ConfigError::file_read_error(&resolved_path, e.to_string()))?
        {
            let path = entry.path();

            // Skip directories and non-config files
            if path.is_dir() {
                continue;
            }

            let format = ConfigFormat::from_path(&path);
            if matches!(format, ConfigFormat::Unknown(_)) {
                continue;
            }

            // Load file
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| ConfigError::file_read_error(&path, e.to_string()))?;

            let value = parse_content(&content, format, &path)?;

            // Use filename without extension as key
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                result.insert(stem.to_string(), value);
            }
        }

        Ok(serde_json::Value::Object(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "yaml")]
    #[test]
    fn parse_simple_yaml() {
        let yaml = "key: value\nnested:\n  inner: 42\n";
        let result = parse_content(yaml, ConfigFormat::Yaml, Path::new("test.yaml")).unwrap();
        assert_eq!(result["key"], "value");
        assert_eq!(result["nested"]["inner"], 42);
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn parse_yaml_arrays() {
        let yaml = "items:\n  - one\n  - two\n  - three\n";
        let result = parse_content(yaml, ConfigFormat::Yaml, Path::new("test.yaml")).unwrap();
        let items = result["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], "one");
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn parse_yaml_anchors_and_aliases() {
        let yaml = "defaults: &defaults\n  timeout: 30\nother:\n  ref_val: *defaults\n";
        let result = parse_content(yaml, ConfigFormat::Yaml, Path::new("test.yaml")).unwrap();
        assert_eq!(result["defaults"]["timeout"], 30);
        assert_eq!(result["other"]["ref_val"]["timeout"], 30);
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn parse_malformed_yaml_returns_error() {
        let yaml = "key: value\n  bad indent: here\n";
        let result = parse_content(yaml, ConfigFormat::Yaml, Path::new("test.yaml"));
        assert!(result.is_err());
    }

    #[cfg(not(feature = "yaml"))]
    #[test]
    fn yaml_disabled_returns_format_not_supported() {
        let result = parse_content("key: value", ConfigFormat::Yaml, Path::new("test.yaml"));
        assert!(result.is_err());
    }
}
