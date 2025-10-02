//! Main configuration container

use super::{ConfigError, ConfigResult, ConfigSource, SourceMetadata};
use super::{ConfigLoader, ConfigValidator, ConfigWatcher};
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use nebula_value::Value as NebulaValue;

/// Main configuration container
#[derive(Clone)]
pub struct Config {
    /// Configuration data
    data: Arc<RwLock<serde_json::Value>>,

    /// Configuration sources
    sources: Vec<ConfigSource>,

    /// Source metadata
    metadata: Arc<DashMap<ConfigSource, SourceMetadata>>,

    /// Configuration loader
    loader: Arc<dyn ConfigLoader>,

    /// Configuration validator
    validator: Option<Arc<dyn ConfigValidator>>,

    /// Configuration watcher
    watcher: Option<Arc<dyn ConfigWatcher>>,

    /// Hot reload enabled
    hot_reload: bool,
}

impl Config {
    /// Create new config (internal use only, use ConfigBuilder)
    pub(crate) fn new(
        data: serde_json::Value,
        sources: Vec<ConfigSource>,
        loader: Arc<dyn ConfigLoader>,
        validator: Option<Arc<dyn ConfigValidator>>,
        watcher: Option<Arc<dyn ConfigWatcher>>,
        hot_reload: bool,
    ) -> Self {
        Self {
            data: Arc::new(RwLock::new(data)),
            sources,
            metadata: Arc::new(DashMap::new()),
            loader,
            validator,
            watcher,
            hot_reload,
        }
    }

    /// Get entire configuration as typed value
    pub async fn get_all<T>(&self) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        serde_json::from_value(data.clone()).map_err(|e| {
            ConfigError::type_error(
                e.to_string(),
                std::any::type_name::<T>(),
                "JSON value",
            )
        })
    }

    /// Get configuration value by path
    pub async fn get<T>(&self, path: &str) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        let value = self.get_nested_value(&data, path)?;
        serde_json::from_value(value.clone()).map_err(|e| {
            ConfigError::type_error(
                e.to_string(),
                std::any::type_name::<T>(),
                "JSON value",
            )
        })
    }

    /// Get configuration value by path (alias for get)
    pub async fn get_path<T>(&self, path: &str) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        self.get(path).await
    }

    /// Get configuration value by path with default
    pub async fn get_or<T>(&self, path: &str, default: T) -> T
    where
        T: DeserializeOwned,
    {
        self.get(path).await.unwrap_or(default)
    }

    /// Get configuration value by path or default
    pub async fn get_or_else<T, F>(&self, path: &str, default_fn: F) -> T
    where
        T: DeserializeOwned,
        F: FnOnce() -> T,
    {
        self.get(path).await.unwrap_or_else(|_| default_fn())
    }

    /// Check if configuration has a path
    pub async fn has(&self, path: &str) -> bool {
        let data = self.data.read().await;
        self.get_nested_value(&data, path).is_ok()
    }

    /// Try to get configuration value by path, returning None on error
    pub async fn get_opt<T>(&self, path: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        self.get(path).await.ok()
    }

    /// Get all configuration keys at a path
    pub async fn keys(&self, path: Option<&str>) -> ConfigResult<Vec<String>> {
        let data = self.data.read().await;
        let value = if let Some(path) = path {
            self.get_nested_value(&data, path)?
        } else {
            &*data
        };

        match value {
            serde_json::Value::Object(obj) => Ok(obj.keys().cloned().collect()),
            _ => Err(ConfigError::type_error(
                "Path does not point to an object",
                "Object",
                value.to_string(),
            )),
        }
    }

    /// Get raw JSON value at path
    pub async fn get_raw(&self, path: Option<&str>) -> ConfigResult<serde_json::Value> {
        let data = self.data.read().await;

        if let Some(path) = path {
            Ok(self.get_nested_value(&data, path)?.clone())
        } else {
            Ok(data.clone())
        }
    }

    /// Reload configuration from all sources
    pub async fn reload(&self) -> ConfigResult<()> {
        nebula_log::info!("Reloading configuration from {} sources", self.sources.len());

        let mut merged_data = serde_json::Value::Object(serde_json::Map::new());

        // Load from all sources in priority order (reverse order)
        let mut sources = self.sources.clone();
        sources.sort_by_key(|s| std::cmp::Reverse(s.priority()));

        for source in &sources {
            match self.loader.load(source).await {
                Ok(data) => {
                    nebula_log::debug!("Loaded configuration from source: {}", source);

                    // Update metadata
                    if let Ok(metadata) = self.loader.metadata(source).await {
                        self.metadata.insert(source.clone(), metadata);
                    }

                    // Merge data
                    self.merge_values(&mut merged_data, data)?;
                }
                Err(e) => {
                    nebula_log::warn!("Failed to load from source {}: {}", source, e);

                    // Decide whether to fail or continue based on source type
                    if !source.is_optional() {
                        return Err(e);
                    }
                }
            }
        }

        // Validate if validator is present
        if let Some(validator) = &self.validator {
            nebula_log::debug!("Validating configuration");
            validator.validate(&merged_data).await?;
        }

        // Update configuration data
        {
            let mut data = self.data.write().await;
            *data = merged_data;
        }

        nebula_log::info!("Configuration reloaded successfully");
        Ok(())
    }

    /// Get source metadata
    pub fn get_metadata(&self, source: &ConfigSource) -> Option<SourceMetadata> {
        self.metadata.get(source).map(|entry| entry.clone())
    }

    /// Get all source metadata
    pub fn get_all_metadata(&self) -> HashMap<ConfigSource, SourceMetadata> {
        self.metadata
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Get configuration sources
    pub fn sources(&self) -> &[ConfigSource] {
        &self.sources
    }

    /// Start watching for configuration changes (if hot reload is enabled)
    pub async fn start_watching(&self) -> ConfigResult<()> {
        if !self.hot_reload {
            nebula_log::debug!("Hot reload is disabled, skipping watch setup");
            return Ok(());
        }

        if let Some(watcher) = &self.watcher {
            nebula_log::info!("Starting configuration watcher");
            watcher.start_watching(&self.sources).await?;
        } else {
            nebula_log::debug!("No watcher configured");
        }

        Ok(())
    }

    /// Stop watching for configuration changes
    pub async fn stop_watching(&self) -> ConfigResult<()> {
        if let Some(watcher) = &self.watcher {
            nebula_log::info!("Stopping configuration watcher");
            watcher.stop_watching().await?;
        }

        Ok(())
    }

    /// Check if watching for changes
    pub fn is_watching(&self) -> bool {
        self.watcher
            .as_ref()
            .map(|w| w.is_watching())
            .unwrap_or(false)
    }

    /// Get nested value from JSON using dot notation
    fn get_nested_value<'a>(
        &self,
        value: &'a serde_json::Value,
        path: &str,
    ) -> ConfigResult<&'a serde_json::Value> {
        if path.is_empty() {
            return Ok(value);
        }

        let parts: Vec<&str> = path.split('.').collect();
        let mut current = value;

        for (i, part) in parts.iter().enumerate() {
            match current {
                serde_json::Value::Object(obj) => {
                    current = obj.get(*part).ok_or_else(|| {
                        ConfigError::path_error(
                            format!("Key '{}' not found", part),
                            path.to_string(),
                        )
                    })?;
                }
                serde_json::Value::Array(arr) => {
                    let index: usize = part.parse().map_err(|_| {
                        ConfigError::path_error(
                            format!("Invalid array index '{}'", part),
                            path.to_string(),
                        )
                    })?;
                    current = arr.get(index).ok_or_else(|| {
                        ConfigError::path_error(
                            format!("Array index {} out of bounds (size: {})", index, arr.len()),
                            path.to_string(),
                        )
                    })?;
                }
                _ => {
                    let remaining_path = parts[i..].join(".");
                    return Err(ConfigError::path_error(
                        format!("Cannot index into {} with '{}'",
                                match current {
                                    serde_json::Value::Null => "null",
                                    serde_json::Value::Bool(_) => "boolean",
                                    serde_json::Value::Number(_) => "number",
                                    serde_json::Value::String(_) => "string",
                                    _ => "value",
                                },
                                remaining_path
                        ),
                        path.to_string(),
                    ));
                }
            }
        }

        Ok(current)
    }

    /// Set nested value in JSON using dot notation
    fn set_nested_value(
        &self,
        value: &mut serde_json::Value,
        path: &str,
        new_value: serde_json::Value,
    ) -> ConfigResult<()> {
        if path.is_empty() {
            *value = new_value;
            return Ok(());
        }

        let parts: Vec<&str> = path.split('.').collect();
        let mut current = value;

        // Navigate to the parent of the target key
        for part in &parts[..parts.len() - 1] {
            match current {
                serde_json::Value::Object(obj) => {
                    current = obj.entry(part.to_string())
                        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                }
                serde_json::Value::Array(arr) => {
                    let index: usize = part.parse().map_err(|_| {
                        ConfigError::path_error(
                            format!("Invalid array index '{}'", part),
                            path.to_string(),
                        )
                    })?;

                    // Extend array if necessary
                    while arr.len() <= index {
                        arr.push(serde_json::Value::Null);
                    }

                    current = &mut arr[index];
                }
                _ => {
                    return Err(ConfigError::path_error(
                        format!("Cannot navigate into {} type",
                            match current {
                                serde_json::Value::Null => "null",
                                serde_json::Value::Bool(_) => "boolean",
                                serde_json::Value::Number(_) => "number",
                                serde_json::Value::String(_) => "string",
                                _ => "value",
                            }
                        ),
                        path.to_string(),
                    ));
                }
            }
        }

        // Set the final value
        let final_key = parts[parts.len() - 1];
        match current {
            serde_json::Value::Object(obj) => {
                obj.insert(final_key.to_string(), new_value);
            }
            serde_json::Value::Array(arr) => {
                let index: usize = final_key.parse().map_err(|_| {
                    ConfigError::path_error(
                        format!("Invalid array index '{}'", final_key),
                        path.to_string(),
                    )
                })?;

                // Extend array if necessary
                while arr.len() <= index {
                    arr.push(serde_json::Value::Null);
                }

                arr[index] = new_value;
            }
            _ => {
                return Err(ConfigError::path_error(
                    format!("Cannot set value in {} type",
                        match current {
                            serde_json::Value::Null => "null",
                            serde_json::Value::Bool(_) => "boolean",
                            serde_json::Value::Number(_) => "number",
                            serde_json::Value::String(_) => "string",
                            _ => "value",
                        }
                    ),
                    path.to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Merge two JSON values
    pub(crate) fn merge_values(
        &self,
        target: &mut serde_json::Value,
        source: serde_json::Value,
    ) -> ConfigResult<()> {
        match (target, source) {
            (serde_json::Value::Object(target_obj), serde_json::Value::Object(source_obj)) => {
                for (key, value) in source_obj {
                    if let Some(existing) = target_obj.get_mut(&key) {
                        self.merge_values(existing, value)?;
                    } else {
                        target_obj.insert(key, value);
                    }
                }
            }
            (target, source) => {
                *target = source;
            }
        }
        Ok(())
    }

    // ==================== Dynamic Value Integration ====================

    /// Get entire configuration as dynamic value
    pub async fn as_value(&self) -> NebulaValue {
        let data = self.data.read().await;
        json_to_value(&data)
    }

    /// Get configuration value by path as dynamic value
    pub async fn get_value(&self, path: &str) -> ConfigResult<NebulaValue> {
        let data = self.data.read().await;
        let json_value = self.get_nested_value(&data, path)?;
        Ok(json_to_value(json_value))
    }

    /// Set configuration value from dynamic value
    pub async fn set_value(&self, path: &str, value: NebulaValue) -> ConfigResult<()> {
        let json_value = value_to_json(value)?;
        self.set_json_path(path, json_value).await
    }

    /// Set configuration value by path with JSON value
    pub async fn set_json_path(&self, path: &str, value: serde_json::Value) -> ConfigResult<()> {
        let mut data = self.data.write().await;
        self.set_nested_value(&mut data, path, value)?;
        Ok(())
    }

    /// Get typed configuration with automatic deserialization
    pub async fn get_typed<T>(&self, path: &str) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        let json_value = self.get_nested_value(&data, path)?;
        serde_json::from_value(json_value.clone()).map_err(|e| {
            ConfigError::type_error(
                format!("Failed to deserialize: {}", e),
                std::any::type_name::<T>(),
                "JSON value",
            )
        })
    }

    /// Set typed configuration with automatic serialization
    pub async fn set_typed<T>(&self, path: &str, value: T) -> ConfigResult<()>
    where
        T: serde::Serialize,
    {
        let json_value = serde_json::to_value(value).map_err(|e| {
            ConfigError::type_error(
                format!("Failed to serialize: {}", e),
                "JSON value",
                std::any::type_name::<T>(),
            )
        })?;
        self.set_json_path(path, json_value).await
    }

    /// Get all configuration as flat key-value map
    pub async fn flatten(&self) -> HashMap<String, NebulaValue> {
        let value = self.as_value().await;
        flatten_value("", &value)
    }

    /// Merge configuration from dynamic value
    pub async fn merge(&self, value: NebulaValue) -> ConfigResult<()> {
        let json_value = value_to_json(value)?;
        let mut data = self.data.write().await;
        self.merge_values(&mut data, json_value)
    }
}

/// Convert serde_json::Value to NebulaValue
fn json_to_value(json: &serde_json::Value) -> NebulaValue {
    match json {
        serde_json::Value::Null => NebulaValue::Null,
        serde_json::Value::Bool(b) => NebulaValue::boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                NebulaValue::integer(i)
            } else if let Some(f) = n.as_f64() {
                NebulaValue::float(f)
            } else {
                NebulaValue::Null
            }
        }
        serde_json::Value::String(s) => NebulaValue::text(s.clone()),
        serde_json::Value::Array(arr) => {
            // Array::from accepts Vec<serde_json::Value>, not Vec<NebulaValue>
            NebulaValue::Array(nebula_value::Array::from(arr.clone()))
        }
        serde_json::Value::Object(obj) => {
            let mut map = nebula_value::Object::new();
            for (k, v) in obj {
                // Object stores serde_json::Value internally, not NebulaValue
                map = map.insert(k.clone(), v.clone());
            }
            NebulaValue::Object(map)
        }
    }
}

/// Convert NebulaValue to serde_json::Value
fn value_to_json(value: NebulaValue) -> ConfigResult<serde_json::Value> {
    match value {
        NebulaValue::Null => Ok(serde_json::Value::Null),
        NebulaValue::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        NebulaValue::Integer(i) => Ok(serde_json::Value::Number(i.value().into())),
        NebulaValue::Float(f) => {
            serde_json::Number::from_f64(f.value())
                .map(serde_json::Value::Number)
                .ok_or_else(|| ConfigError::type_error("Invalid float value", "valid float", "NaN/Infinity"))
        }
        NebulaValue::Text(t) => Ok(serde_json::Value::String(t.to_string())),
        NebulaValue::Array(arr) => {
            // Array stores serde_json::Value internally
            let items: Vec<_> = arr.iter()
                .map(|v| v.clone())
                .collect();
            Ok(serde_json::Value::Array(items))
        }
        NebulaValue::Object(obj) => {
            let mut map = serde_json::Map::new();
            for (k, v) in obj.entries() {
                map.insert(k.clone(), v.clone());
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Err(ConfigError::type_error("Unsupported NebulaValue type for JSON conversion", "basic types", "complex type")),
    }
}

/// Flatten NebulaValue into a map with dot-notation keys
fn flatten_value(prefix: &str, value: &NebulaValue) -> HashMap<String, NebulaValue> {
    let mut map = HashMap::new();

    match value {
        NebulaValue::Object(obj) => {
            for (key, val) in obj.entries() {
                let full_key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };

                // Convert serde_json::Value to NebulaValue
                let nebula_val = json_to_value(val);
                let nested = flatten_value(&full_key, &nebula_val);
                map.extend(nested);
            }
        }
        NebulaValue::Array(arr) => {
            for (index, val) in arr.iter().enumerate() {
                let full_key = if prefix.is_empty() {
                    index.to_string()
                } else {
                    format!("{}[{}]", prefix, index)
                };

                // Convert serde_json::Value to NebulaValue
                let nebula_val = json_to_value(val);
                let nested = flatten_value(&full_key, &nebula_val);
                map.extend(nested);
            }
        }
        _ => {
            map.insert(prefix.to_string(), value.clone());
        }
    }

    map
}

// Cleanup on drop
impl Drop for Config {
    fn drop(&mut self) {
        // Try to stop watching if possible
        if let Some(watcher) = &self.watcher {
            if watcher.is_watching() {
                // We can't await in drop, so we just log
                nebula_log::debug!("Config dropped while still watching");
            }
        }
    }
}