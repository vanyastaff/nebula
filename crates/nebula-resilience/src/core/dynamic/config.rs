//! Dynamic configuration support using nebula-value

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::config::{ConfigError, ConfigResult, ResilienceConfig};
use nebula_value::{Object, Value};

/// Get current timestamp as ISO 8601 string
fn current_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    format!("{}.{:03}Z", now.as_secs(), now.subsec_millis())
}

/// Dynamic configuration container that can hold any resilience configuration
#[derive(Debug, Clone)]
pub struct DynamicConfig {
    /// Configuration values stored as nebula-value
    values: Object,
    /// Configuration schema version for compatibility tracking
    schema_version: String,
    /// Last update timestamp for change tracking
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
    #[must_use]
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
        // Update timestamp when configuration changes
        self.last_updated = Some(current_timestamp());
        Ok(())
    }

    /// Get the schema version
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Get the last update timestamp
    #[must_use]
    pub fn last_updated(&self) -> Option<&str> {
        self.last_updated.as_deref()
    }

    /// Set a new value and update timestamp
    pub fn set_value_tracked(&mut self, path: &str, value: Value) -> ConfigResult<()> {
        self.set_value(path, value)?;
        self.last_updated = Some(current_timestamp());
        Ok(())
    }

    /// Convert to a flat key-value map for debugging
    #[must_use]
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
            // Object already stores nebula Values directly
            Ok(obj.insert(path[0].to_string(), value))
        } else {
            let key = path[0];
            let remaining = &path[1..];

            // Get nested value or create empty object
            let nested = obj
                .get(key)
                .cloned()
                .unwrap_or(Value::Object(Object::new()));

            match nested {
                Value::Object(nested_obj) => {
                    let updated_nested = self.set_nested_value(&nested_obj, remaining, value)?;
                    Ok(obj.insert(key.to_string(), Value::Object(updated_nested)))
                }
                _ => Err(ConfigError::validation(format!(
                    "Path '{key}' exists but is not an object"
                ))),
            }
        }
    }

    fn get_nested_value(&self, obj: &Object, path: &[&str]) -> ConfigResult<Value> {
        if path.is_empty() {
            return Err(ConfigError::validation("Empty path not allowed"));
        }

        let key = path[0];
        let value = obj
            .get(key)
            .ok_or_else(|| ConfigError::not_found("config", key))?;

        if path.len() == 1 {
            Ok(value.clone())
        } else {
            // Navigate deeper into nested object
            match value {
                Value::Object(nested_obj) => self.get_nested_value(nested_obj, &path[1..]),
                _ => Err(ConfigError::validation(format!(
                    "Path '{key}' is not an object"
                ))),
            }
        }
    }

    fn flatten_object(&self, obj: &Object, prefix: &str, map: &mut HashMap<String, String>) {
        for (key, value) in obj.entries() {
            let full_key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };

            // Check if it's a nested object
            match value {
                Value::Object(nested_obj) => {
                    self.flatten_object(nested_obj, &full_key, map);
                }
                _ => {
                    map.insert(full_key, format!("{value}"));
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
    #[must_use]
    pub fn database() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // Circuit breaker for database
        config
            .set_value("circuit_breaker.failure_threshold", Value::integer(5))
            .expect("preset configuration should be valid");
        config
            .set_value("circuit_breaker.reset_timeout", Value::text("60s"))
            .expect("preset configuration should be valid");
        config
            .set_value(
                "circuit_breaker.half_open_max_operations",
                Value::integer(3),
            )
            .expect("preset configuration should be valid");

        // Retry for database
        config
            .set_value("retry.max_attempts", Value::integer(3))
            .expect("preset configuration should be valid");
        config
            .set_value("retry.base_delay", Value::text("100ms"))
            .expect("preset configuration should be valid");
        config
            .set_value("retry.max_delay", Value::text("5s"))
            .expect("preset configuration should be valid");

        // Timeout for database
        config
            .set_value("timeout.duration", Value::text("30s"))
            .expect("preset configuration should be valid");

        config
    }

    /// Get HTTP API configuration
    #[must_use]
    pub fn http_api() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // Circuit breaker for HTTP
        config
            .set_value("circuit_breaker.failure_threshold", Value::integer(3))
            .expect("preset configuration should be valid");
        config
            .set_value("circuit_breaker.reset_timeout", Value::text("30s"))
            .expect("preset configuration should be valid");
        config
            .set_value(
                "circuit_breaker.half_open_max_operations",
                Value::integer(2),
            )
            .expect("preset configuration should be valid");

        // Retry for HTTP
        config
            .set_value("retry.max_attempts", Value::integer(3))
            .expect("preset configuration should be valid");
        config
            .set_value("retry.base_delay", Value::text("1s"))
            .expect("preset configuration should be valid");
        config
            .set_value("retry.max_delay", Value::text("10s"))
            .expect("preset configuration should be valid");

        // Timeout for HTTP
        config
            .set_value("timeout.duration", Value::text("10s"))
            .expect("preset configuration should be valid");

        // Rate limiting for HTTP
        config
            .set_value("rate_limit.requests_per_second", Value::integer(100))
            .expect("preset configuration should be valid");
        config
            .set_value("rate_limit.burst", Value::integer(20))
            .expect("preset configuration should be valid");

        config
    }

    /// Get file I/O configuration
    #[must_use]
    pub fn file_io() -> DynamicConfig {
        let mut config = DynamicConfig::new();

        // No circuit breaker for file I/O (usually not needed)

        // Light retry for file I/O
        config
            .set_value("retry.max_attempts", Value::integer(2))
            .expect("preset configuration should be valid");
        config
            .set_value("retry.base_delay", Value::text("500ms"))
            .expect("preset configuration should be valid");
        config
            .set_value("retry.max_delay", Value::text("2s"))
            .expect("preset configuration should be valid");

        // Generous timeout for file I/O
        config
            .set_value("timeout.duration", Value::text("60s"))
            .expect("preset configuration should be valid");

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
