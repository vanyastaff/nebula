//! Dynamic configuration support using nebula-value

use std::collections::HashMap;

use nebula_value::{Object, Value};
// Import extension traits for ergonomic conversions
use crate::core::config::{ConfigError, ConfigResult, ResilienceConfig};
use nebula_value::{JsonValueExt, ValueRefExt};

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
            // Convert nebula_value::Value to serde_json::Value for storage
            let json_value = value.to_json();
            Ok(obj.insert(path[0].to_string(), json_value))
        } else {
            let key = path[0];
            let remaining = &path[1..];

            // Get nested value or create empty object
            let nested_json = obj
                .get(key)
                .cloned()
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

            // Convert to nebula Value for processing using extension trait
            let nested = nested_json.to_nebula_value_or_null();

            match nested {
                Value::Object(nested_obj) => {
                    let updated_nested = self.set_nested_value(&nested_obj, remaining, value)?;
                    let updated_json = serde_json::Value::Object(
                        updated_nested
                            .entries()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    );
                    Ok(obj.insert(key.to_string(), updated_json))
                }
                _ => Err(ConfigError::validation(format!(
                    "Path '{}' exists but is not an object",
                    key
                ))),
            }
        }
    }

    fn get_nested_value(&self, obj: &Object, path: &[&str]) -> ConfigResult<Value> {
        if path.is_empty() {
            return Err(ConfigError::validation("Empty path not allowed"));
        }

        let key = path[0];
        let json_value = obj
            .get(key)
            .ok_or_else(|| ConfigError::not_found("config", key))?;

        if path.len() == 1 {
            Ok(json_value.to_nebula_value_or_null())
        } else {
            // Convert serde_json::Value to nebula Value for matching
            let value = json_value.to_nebula_value_or_null();
            match value {
                Value::Object(nested_obj) => self.get_nested_value(&nested_obj, &path[1..]),
                _ => Err(ConfigError::validation(format!(
                    "Path '{}' is not an object",
                    key
                ))),
            }
        }
    }

    fn flatten_object(&self, obj: &Object, prefix: &str, map: &mut HashMap<String, String>) {
        for (key, json_value) in obj.entries() {
            let full_key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", prefix, key)
            };

            // Convert to nebula Value to check if it's an object
            match json_value {
                serde_json::Value::Object(nested_map) => {
                    let nested_obj = Object::from_iter(nested_map.clone());
                    self.flatten_object(&nested_obj, &full_key, map);
                }
                _ => {
                    map.insert(full_key, format!("{}", json_value));
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
        config
            .set_value("circuit_breaker.failure_threshold", Value::integer(5))
            .unwrap();
        config
            .set_value("circuit_breaker.reset_timeout", Value::text("60s"))
            .unwrap();
        config
            .set_value(
                "circuit_breaker.half_open_max_operations",
                Value::integer(3),
            )
            .unwrap();

        // Retry for database
        config
            .set_value("retry.max_attempts", Value::integer(3))
            .unwrap();
        config
            .set_value("retry.base_delay", Value::text("100ms"))
            .unwrap();
        config
            .set_value("retry.max_delay", Value::text("5s"))
            .unwrap();

        // Timeout for database
        config
            .set_value("timeout.duration", Value::text("30s"))
            .unwrap();

        config
    }

    /// Get HTTP API configuration
    pub fn http_api() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // Circuit breaker for HTTP
        config
            .set_value("circuit_breaker.failure_threshold", Value::integer(3))
            .unwrap();
        config
            .set_value("circuit_breaker.reset_timeout", Value::text("30s"))
            .unwrap();
        config
            .set_value(
                "circuit_breaker.half_open_max_operations",
                Value::integer(2),
            )
            .unwrap();

        // Retry for HTTP
        config
            .set_value("retry.max_attempts", Value::integer(3))
            .unwrap();
        config
            .set_value("retry.base_delay", Value::text("1s"))
            .unwrap();
        config
            .set_value("retry.max_delay", Value::text("10s"))
            .unwrap();

        // Timeout for HTTP
        config
            .set_value("timeout.duration", Value::text("10s"))
            .unwrap();

        // Rate limiting for HTTP
        config
            .set_value("rate_limit.requests_per_second", Value::integer(100))
            .unwrap();
        config
            .set_value("rate_limit.burst", Value::integer(20))
            .unwrap();

        config
    }

    /// Get file I/O configuration
    pub fn file_io() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // No circuit breaker for file I/O (usually not needed)

        // Light retry for file I/O
        config
            .set_value("retry.max_attempts", Value::integer(2))
            .unwrap();
        config
            .set_value("retry.base_delay", Value::text("500ms"))
            .unwrap();
        config
            .set_value("retry.max_delay", Value::text("2s"))
            .unwrap();

        // Generous timeout for file I/O
        config
            .set_value("timeout.duration", Value::text("60s"))
            .unwrap();

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_config_basic() {
        let mut config = DynamicConfig::new();

        config.set_value("test.key", Value::text("value")).unwrap();
        let retrieved = config.get_value("test.key").unwrap();

        assert_eq!(retrieved, Value::text("value"));
    }

    #[test]
    fn test_dynamic_config_nested() {
        let mut config = DynamicConfig::new();

        config
            .set_value("level1.level2.key", Value::integer(42))
            .unwrap();
        let retrieved = config.get_value("level1.level2.key").unwrap();

        assert_eq!(retrieved, Value::integer(42));
    }

    #[test]
    fn test_presets() {
        let db_config = ResiliencePresets::database();
        let threshold = db_config
            .get_value("circuit_breaker.failure_threshold")
            .unwrap();
        assert_eq!(threshold, Value::integer(5));

        let http_config = ResiliencePresets::http_api();
        let timeout = http_config.get_value("timeout.duration").unwrap();
        assert_eq!(timeout, Value::text("10s"));
    }
}
