//! Environment variable configuration loader

use crate::core::{
    ConfigError, ConfigFormat, ConfigLoader, ConfigResult, ConfigSource, SourceMetadata,
};
use async_trait::async_trait;
use std::collections::HashMap;

/// Environment variable loader
#[derive(Debug, Clone)]
pub struct EnvLoader {
    /// Environment variable prefix
    pub prefix: Option<String>,

    /// Separator for nested keys
    pub separator: String,

    /// Case sensitivity
    pub case_sensitive: bool,

    /// Whether to log sensitive values
    pub log_sensitive: bool,
}

impl EnvLoader {
    /// Create a new environment loader
    pub fn new() -> Self {
        Self {
            prefix: None,
            separator: "_".to_string(),
            case_sensitive: false,
            log_sensitive: false,
        }
    }

    /// Create a new environment loader with prefix
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: Some(prefix.into()),
            separator: "_".to_string(),
            case_sensitive: false,
            log_sensitive: false,
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

    /// Set whether to log sensitive values
    pub fn with_log_sensitive(mut self, log_sensitive: bool) -> Self {
        self.log_sensitive = log_sensitive;
        self
    }

    /// Check if a key is sensitive
    fn is_sensitive_key(&self, key: &str) -> bool {
        let key_lower = key.to_lowercase();
        key_lower.contains("password")
            || key_lower.contains("secret")
            || key_lower.contains("token")
            || key_lower.contains("api_key")
            || key_lower.contains("private")
            || key_lower.contains("credential")
    }

    /// Convert environment variables to nested JSON structure
    fn env_to_json(&self, vars: HashMap<String, String>) -> serde_json::Value {
        let mut result = serde_json::Map::new();

        for (key, value) in vars {
            // Log the key-value pair (hiding sensitive values)
            if self.is_sensitive_key(&key) && !self.log_sensitive {
                nebula_log::trace!("Loading env config: {} = [REDACTED]", key);
            } else {
                nebula_log::trace!("Loading env config: {} = {}", key, value);
            }

            let parts: Vec<&str> = key.split(&self.separator).collect();
            self.insert_nested(&mut result, &parts, value);
        }

        serde_json::Value::Object(result)
    }

    /// Insert value into nested structure
    fn insert_nested(
        &self,
        obj: &mut serde_json::Map<String, serde_json::Value>,
        parts: &[&str],
        value: String,
    ) {
        if parts.is_empty() {
            return;
        }

        if parts.len() == 1 {
            let key = if self.case_sensitive {
                parts[0].to_string()
            } else {
                parts[0].to_lowercase()
            };

            let parsed_value = self.parse_env_value(&value);
            obj.insert(key, parsed_value);
            return;
        }

        let key = if self.case_sensitive {
            parts[0].to_string()
        } else {
            parts[0].to_lowercase()
        };

        let remaining = &parts[1..];

        let nested = obj
            .entry(key)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

        if let serde_json::Value::Object(nested_obj) = nested {
            self.insert_nested(nested_obj, remaining, value);
        }
    }

    /// Parse environment variable value
    fn parse_env_value(&self, value: &str) -> serde_json::Value {
        // Empty string
        if value.is_empty() {
            return serde_json::Value::String(String::new());
        }

        // Try to parse as boolean
        if value.eq_ignore_ascii_case("true") {
            return serde_json::Value::Bool(true);
        }
        if value.eq_ignore_ascii_case("false") {
            return serde_json::Value::Bool(false);
        }

        // Try to parse as integer
        if let Ok(int_val) = value.parse::<i64>() {
            return serde_json::Value::Number(serde_json::Number::from(int_val));
        }

        // Try to parse as float
        if let Ok(float_val) = value.parse::<f64>() {
            if let Some(num) = serde_json::Number::from_f64(float_val) {
                return serde_json::Value::Number(num);
            }
        }

        // Try to parse as JSON (for arrays and objects)
        if (value.starts_with('{') && value.ends_with('}'))
            || (value.starts_with('[') && value.ends_with(']'))
        {
            if let Ok(json_val) = serde_json::from_str(value) {
                return json_val;
            }
        }

        // Parse comma-separated values as array
        if value.contains(',') && !value.starts_with('"') {
            let items: Vec<serde_json::Value> = value
                .split(',')
                .map(|s| self.parse_env_value(s.trim()))
                .collect();
            return serde_json::Value::Array(items);
        }

        // Default to string
        serde_json::Value::String(value.to_string())
    }

    /// Filter environment variables by prefix
    fn filter_vars(&self, prefix: &str) -> HashMap<String, String> {
        std::env::vars()
            .filter_map(|(key, value)| {
                let key_to_check = if self.case_sensitive {
                    key.clone()
                } else {
                    key.to_uppercase()
                };

                let prefix_to_check = if self.case_sensitive {
                    prefix.to_string()
                } else {
                    prefix.to_uppercase()
                };

                if key_to_check.starts_with(&prefix_to_check) {
                    let stripped_key = key_to_check
                        .strip_prefix(&prefix_to_check)
                        .unwrap_or(&key_to_check)
                        .trim_start_matches(&self.separator);

                    if stripped_key.is_empty() {
                        None
                    } else {
                        Some((stripped_key.to_string(), value))
                    }
                } else {
                    None
                }
            })
            .collect()
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
                let vars: HashMap<String, String> = if let Some(ref prefix) = self.prefix {
                    self.filter_vars(prefix)
                } else {
                    std::env::vars().collect()
                };

                if vars.is_empty() {
                    nebula_log::debug!("No environment variables found");
                } else {
                    nebula_log::debug!("Loaded {} environment variables", vars.len());
                }

                Ok(self.env_to_json(vars))
            }
            ConfigSource::EnvWithPrefix(prefix) => {
                let vars = self.filter_vars(prefix);

                if vars.is_empty() {
                    nebula_log::debug!("No environment variables found with prefix: {}", prefix);
                } else {
                    nebula_log::debug!(
                        "Loaded {} environment variables with prefix: {}",
                        vars.len(),
                        prefix
                    );
                }

                Ok(self.env_to_json(vars))
            }
            _ => Err(ConfigError::source_error(
                "EnvLoader does not support this source type",
                source.name(),
            )),
        }
    }

    fn supports(&self, source: &ConfigSource) -> bool {
        matches!(source, ConfigSource::Env | ConfigSource::EnvWithPrefix(_))
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
                source.name(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_value() {
        let loader = EnvLoader::new();

        // Test boolean parsing
        assert_eq!(
            loader.parse_env_value("true"),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            loader.parse_env_value("FALSE"),
            serde_json::Value::Bool(false)
        );

        // Test number parsing
        assert_eq!(
            loader.parse_env_value("42"),
            serde_json::Value::Number(42.into())
        );
        assert_eq!(
            loader.parse_env_value("3.14"),
            serde_json::Value::Number(serde_json::Number::from_f64(3.14).unwrap())
        );

        // Test array parsing
        let array_val = loader.parse_env_value("one,two,three");
        assert!(array_val.is_array());

        // Test JSON parsing
        let json_val = loader.parse_env_value(r#"{"key":"value"}"#);
        assert!(json_val.is_object());

        // Test string fallback
        assert_eq!(
            loader.parse_env_value("hello world"),
            serde_json::Value::String("hello world".to_string())
        );
    }

    #[test]
    fn test_is_sensitive_key() {
        let loader = EnvLoader::new();

        assert!(loader.is_sensitive_key("PASSWORD"));
        assert!(loader.is_sensitive_key("api_key"));
        assert!(loader.is_sensitive_key("SECRET_TOKEN"));
        assert!(loader.is_sensitive_key("private_data"));
        assert!(!loader.is_sensitive_key("USERNAME"));
        assert!(!loader.is_sensitive_key("PORT"));
    }
}
