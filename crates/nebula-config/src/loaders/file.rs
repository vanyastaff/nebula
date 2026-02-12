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

    /// Parse configuration content based on format
    fn parse_content(
        &self,
        content: &str,
        format: ConfigFormat,
        path: &Path,
    ) -> ConfigResult<serde_json::Value> {
        parse_content(content, format, path)
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
        ConfigFormat::Json => serde_json::from_str(content)
            .map_err(|e| ConfigError::parse_error(path, format!("JSON parse error: {}", e))),
        ConfigFormat::Toml => toml::from_str::<serde_json::Value>(content)
            .map_err(|e| ConfigError::parse_error(path, format!("TOML parse error: {}", e))),
        ConfigFormat::Yaml => parse_yaml(content, path),
        ConfigFormat::Ini => parse_ini(content, path),
        ConfigFormat::Properties => parse_properties(content, path),
        _ => Err(ConfigError::format_not_supported(format.to_string())),
    }
}

/// Parse YAML content into JSON
fn parse_yaml(content: &str, path: &Path) -> ConfigResult<serde_json::Value> {
    use yaml_rust2::YamlLoader;

    let docs = YamlLoader::load_from_str(content)
        .map_err(|e| ConfigError::parse_error(path, format!("YAML parse error: {:?}", e)))?;

    if docs.is_empty() {
        return Ok(serde_json::Value::Null);
    }

    yaml_to_json(&docs[0], path)
}

/// Convert YAML value to JSON value
fn yaml_to_json(yaml: &yaml_rust2::Yaml, path: &Path) -> ConfigResult<serde_json::Value> {
    use yaml_rust2::Yaml;

    match yaml {
        Yaml::Real(s) | Yaml::String(s) => {
            if let Ok(num) = s.parse::<f64>()
                && let Some(json_num) = serde_json::Number::from_f64(num)
            {
                return Ok(serde_json::Value::Number(json_num));
            }
            Ok(serde_json::Value::String(s.clone()))
        }
        Yaml::Integer(i) => Ok(serde_json::Value::Number(serde_json::Number::from(*i))),
        Yaml::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Yaml::Array(arr) => {
            let mut json_arr = Vec::with_capacity(arr.len());
            for item in arr {
                json_arr.push(yaml_to_json(item, path)?);
            }
            Ok(serde_json::Value::Array(json_arr))
        }
        Yaml::Hash(hash) => {
            let mut json_obj = serde_json::Map::new();
            for (key, value) in hash {
                let key_str = match key {
                    Yaml::String(s) => s.clone(),
                    Yaml::Integer(i) => i.to_string(),
                    _ => {
                        return Err(ConfigError::parse_error(
                            path,
                            "Invalid key type in YAML hash",
                        ));
                    }
                };
                json_obj.insert(key_str, yaml_to_json(value, path)?);
            }
            Ok(serde_json::Value::Object(json_obj))
        }
        Yaml::Null => Ok(serde_json::Value::Null),
        Yaml::BadValue => Err(ConfigError::parse_error(path, "Bad YAML value encountered")),
        _ => Err(ConfigError::parse_error(path, "Unsupported YAML type")),
    }
}

/// Parse a scalar value from string (bool, int, float, or string)
pub(crate) fn parse_scalar_value(value: &str) -> serde_json::Value {
    // Remove quotes if present
    let value = if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };

    if value.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if let Ok(int_val) = value.parse::<i64>() {
        return serde_json::Value::Number(serde_json::Number::from(int_val));
    }
    if let Ok(float_val) = value.parse::<f64>()
        && let Some(num) = serde_json::Number::from_f64(float_val)
    {
        return serde_json::Value::Number(num);
    }
    serde_json::Value::String(value.to_string())
}

/// Parse INI content into JSON
fn parse_ini(content: &str, path: &Path) -> ConfigResult<serde_json::Value> {
    let mut result = serde_json::Map::new();
    let mut current_section = None;

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();

        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = Some(line[1..line.len() - 1].to_string());
            if let Some(section) = &current_section {
                if !result.contains_key(section) {
                    result.insert(
                        section.clone(),
                        serde_json::Value::Object(serde_json::Map::new()),
                    );
                }
            } else {
                return Err(ConfigError::parse_error(
                    path,
                    "Section header missing name",
                ));
            }
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();
            let parsed_value = parse_scalar_value(value);

            if let Some(ref section) = current_section {
                if let Some(serde_json::Value::Object(section_obj)) = result.get_mut(section) {
                    section_obj.insert(key.to_string(), parsed_value);
                }
            } else {
                result.insert(key.to_string(), parsed_value);
            }
        } else {
            return Err(ConfigError::parse_error(
                path,
                format!("Invalid INI format at line {}", line_num + 1),
            ));
        }
    }

    Ok(serde_json::Value::Object(result))
}

/// Parse properties file content into JSON
fn parse_properties(content: &str, path: &Path) -> ConfigResult<serde_json::Value> {
    let mut result = serde_json::Map::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }

        let separator_pos = line.find('=').or_else(|| line.find(':'));

        if let Some(pos) = separator_pos {
            let key = line[..pos].trim();
            let value = line[pos + 1..].trim();
            insert_property_nested(&mut result, key, value);
        } else {
            return Err(ConfigError::parse_error(
                path,
                format!("Invalid properties format at line {}", line_num + 1),
            ));
        }
    }

    Ok(serde_json::Value::Object(result))
}

/// Insert property with dot-notation key into nested JSON structure
fn insert_property_nested(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &str,
) {
    let parts: Vec<&str> = key.split('.').collect();
    insert_property_recursive(obj, &parts, value);
}

fn insert_property_recursive(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    parts: &[&str],
    value: &str,
) {
    if parts.is_empty() {
        return;
    }
    if parts.len() == 1 {
        obj.insert(parts[0].to_string(), parse_scalar_value(value));
        return;
    }

    let entry = obj
        .entry(parts[0].to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

    if let serde_json::Value::Object(map) = entry {
        insert_property_recursive(map, &parts[1..], value);
    } else {
        *entry = serde_json::Value::Object(serde_json::Map::new());
        if let serde_json::Value::Object(map) = entry {
            insert_property_recursive(map, &parts[1..], value);
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
                self.parse_content(&content, format, &resolved_path)
            }
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
            }
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

            let value = self.parse_content(&content, format, &path)?;

            // Use filename without extension as key
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                result.insert(stem.to_string(), value);
            }
        }

        Ok(serde_json::Value::Object(result))
    }
}
