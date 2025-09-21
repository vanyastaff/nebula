//! Main configuration container

use super::{ConfigError, ConfigResult, ConfigSource, SourceMetadata};
use super::{ConfigLoader, ConfigValidator, ConfigWatcher};
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    pub async fn get<T>(&self) -> ConfigResult<T>
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
    pub async fn get_path<T>(&self, path: &str) -> ConfigResult<T>
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

    /// Get configuration value by path with default
    pub async fn get_path_or<T>(&self, path: &str, default: T) -> T
    where
        T: DeserializeOwned,
    {
        self.get_path(path).await.unwrap_or(default)
    }

    /// Get configuration value by path or default
    pub async fn get_path_or_else<T, F>(&self, path: &str, default_fn: F) -> T
    where
        T: DeserializeOwned,
        F: FnOnce() -> T,
    {
        self.get_path(path).await.unwrap_or_else(|_| default_fn())
    }

    /// Check if configuration has a path
    pub async fn has_path(&self, path: &str) -> bool {
        let data = self.data.read().await;
        self.get_nested_value(&data, path).is_ok()
    }

    /// Try to get configuration value by path, returning None on error
    pub async fn get_opt_path<T>(&self, path: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        self.get_path(path).await.ok()
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