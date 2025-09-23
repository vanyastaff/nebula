//! Dynamic configuration support using nebula-value

use std::collections::HashMap;


use nebula_value::{Value, Object};
use crate::core::config::{ResilienceConfig, ConfigResult, ConfigError};

/// Dynamic configuration container that can hold any resilience configuration
#[derive(Debug, Clone)]
pub struct DynamicConfig {
    /// Configuration values stored as nebula-value
    values: Object,
    /// Configuration schema metadata
    #[allow(dead_code)]
    schema_version: String,
    /// Last update timestamp
    #[allow(dead_code)]
    last_updated: Option<String>,
}

impl Default for DynamicConfig {
    fn default() -> Self {
        Self {
            values: Object::new(),
            schema_version: "1.0".to_string(),
            last_updated: None,
        }
    }
}

impl DynamicConfig {
    /// Create a new dynamic configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a configuration value by path
    pub fn set_value(&mut self, path: &str, value: Value) -> ConfigResult<()> {
        let path_parts: Vec<&str> = path.split('.').collect();
        self.values = self.set_nested_value(&self.values, &path_parts, value)?;
        Ok(())
    }

    /// Get a configuration value by path
    pub fn get_value(&self, path: &str) -> ConfigResult<Value> {
        let path_parts: Vec<&str> = path.split('.').collect();
        self.get_nested_value(&self.values, &path_parts)
    }

    /// Set a typed configuration
    pub fn set_config<T: ResilienceConfig>(&mut self, path: &str, config: &T) -> ConfigResult<()> {
        let value = config.to_value();
        self.set_value(path, value)
    }

    /// Get a typed configuration
    pub fn get_config<T: ResilienceConfig>(&self, path: &str) -> ConfigResult<T> {
        let value = self.get_value(path)?;
        T::from_value(&value)
    }

    /// Merge with another dynamic configuration
    pub fn merge(&mut self, other: &DynamicConfig) -> ConfigResult<()> {
        self.values = self.values.merge(&other.values);
        Ok(())
    }

    /// Convert to a flat key-value map for debugging
    pub fn to_flat_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        self.flatten_object(&self.values, "", &mut map);
        map
    }

    fn set_nested_value(&self, obj: &Object, path: &[&str], value: Value) -> ConfigResult<Object> {
        if path.is_empty() {
            return Err(ConfigError::validation("Empty path not allowed"));
        }

        if path.len() == 1 {
            Ok(obj.insert(path[0].to_string(), value))
        } else {
            let key = path[0];
            let remaining = &path[1..];

            let nested = obj.get(key)
                .cloned()
                .unwrap_or(Value::Object(Object::new()));

            match nested {
                Value::Object(nested_obj) => {
                    let updated_nested = self.set_nested_value(&nested_obj, remaining, value)?;
                    Ok(obj.insert(key.to_string(), Value::Object(updated_nested)))
                }
                _ => Err(ConfigError::validation(format!("Path '{}' exists but is not an object", key)))
            }
        }
    }

    fn get_nested_value(&self, obj: &Object, path: &[&str]) -> ConfigResult<Value> {
        if path.is_empty() {
            return Err(ConfigError::validation("Empty path not allowed"));
        }

        let key = path[0];
        let value = obj.get(key)
            .ok_or_else(|| ConfigError::not_found("config", key))?;

        if path.len() == 1 {
            Ok(value.clone())
        } else {
            match value {
                Value::Object(nested_obj) => {
                    self.get_nested_value(nested_obj, &path[1..])
                }
                _ => Err(ConfigError::validation(format!("Path '{}' is not an object", key)))
            }
        }
    }

    fn flatten_object(&self, obj: &Object, prefix: &str, map: &mut HashMap<String, String>) {
        for (key, value) in obj {
            let full_key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", prefix, key)
            };

            match value {
                Value::Object(nested) => {
                    self.flatten_object(nested, &full_key, map);
                }
                _ => {
                    map.insert(full_key, format!("{}", value));
                }
            }
        }
    }
}

/// Helper trait for types that can be converted to/from dynamic configuration
pub trait DynamicConfigurable: ResilienceConfig {
    /// Convert to dynamic configuration
    fn to_dynamic(&self) -> DynamicConfig {
        let mut dynamic = DynamicConfig::new();
        dynamic.set_config("root", self).unwrap_or_default();
        dynamic
    }

    /// Create from dynamic configuration
    fn from_dynamic(dynamic: &DynamicConfig) -> ConfigResult<Self> {
        dynamic.get_config("root")
    }
}

// Blanket implementation for all ResilienceConfig types
impl<T: ResilienceConfig> DynamicConfigurable for T {}

/// Configuration preset for common resilience patterns
pub struct ResiliencePresets;

impl ResiliencePresets {
    /// Get database operation configuration
    pub fn database() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // Circuit breaker for database
        config.set_value("circuit_breaker.failure_threshold", Value::from(5)).unwrap();
        config.set_value("circuit_breaker.reset_timeout", Value::from("60s")).unwrap();
        config.set_value("circuit_breaker.half_open_max_operations", Value::from(3)).unwrap();

        // Retry for database
        config.set_value("retry.max_attempts", Value::from(3)).unwrap();
        config.set_value("retry.base_delay", Value::from("100ms")).unwrap();
        config.set_value("retry.max_delay", Value::from("5s")).unwrap();

        // Timeout for database
        config.set_value("timeout.duration", Value::from("30s")).unwrap();

        config
    }

    /// Get HTTP API configuration
    pub fn http_api() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // Circuit breaker for HTTP
        config.set_value("circuit_breaker.failure_threshold", Value::from(3)).unwrap();
        config.set_value("circuit_breaker.reset_timeout", Value::from("30s")).unwrap();
        config.set_value("circuit_breaker.half_open_max_operations", Value::from(2)).unwrap();

        // Retry for HTTP
        config.set_value("retry.max_attempts", Value::from(3)).unwrap();
        config.set_value("retry.base_delay", Value::from("1s")).unwrap();
        config.set_value("retry.max_delay", Value::from("10s")).unwrap();

        // Timeout for HTTP
        config.set_value("timeout.duration", Value::from("10s")).unwrap();

        // Rate limiting for HTTP
        config.set_value("rate_limit.requests_per_second", Value::from(100)).unwrap();
        config.set_value("rate_limit.burst", Value::from(20)).unwrap();

        config
    }

    /// Get file I/O configuration
    pub fn file_io() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // No circuit breaker for file I/O (usually not needed)

        // Light retry for file I/O
        config.set_value("retry.max_attempts", Value::from(2)).unwrap();
        config.set_value("retry.base_delay", Value::from("500ms")).unwrap();
        config.set_value("retry.max_delay", Value::from("2s")).unwrap();

        // Generous timeout for file I/O
        config.set_value("timeout.duration", Value::from("60s")).unwrap();

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_config_basic() {
        let mut config = DynamicConfig::new();

        config.set_value("test.key", Value::from("value")).unwrap();
        let retrieved = config.get_value("test.key").unwrap();

        assert_eq!(retrieved, Value::from("value"));
    }

    #[test]
    fn test_dynamic_config_nested() {
        let mut config = DynamicConfig::new();

        config.set_value("level1.level2.key", Value::from(42)).unwrap();
        let retrieved = config.get_value("level1.level2.key").unwrap();

        assert_eq!(retrieved, Value::from(42));
    }

    #[test]
    fn test_presets() {
        let db_config = ResiliencePresets::database();
        let threshold = db_config.get_value("circuit_breaker.failure_threshold").unwrap();
        assert_eq!(threshold, Value::from(5));

        let http_config = ResiliencePresets::http_api();
        let timeout = http_config.get_value("timeout.duration").unwrap();
        assert_eq!(timeout, Value::from("10s"));
    }
}