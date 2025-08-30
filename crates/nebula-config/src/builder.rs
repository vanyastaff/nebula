//! Configuration builder and main Config struct

use crate::{
    ConfigError, ConfigResult, ConfigSource, ConfigLoader, ConfigValidator, 
    ConfigWatcher, CompositeLoader, SourceMetadata
};
use async_trait::async_trait;
use serde::{Deserialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::RwLock;

/// Main configuration container
#[derive(Debug)]
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
    /// Get configuration value by path
    pub async fn get<T>(&self) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        serde_json::from_value(data.clone())
            .map_err(|e| ConfigError::type_error(
                e.to_string(),
                std::any::type_name::<T>(),
                "JSON value"
            ))
    }
    
    /// Get configuration value by path
    pub async fn get_path<T>(&self, path: &str) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        let value = self.get_nested_value(&data, path)?;
        serde_json::from_value(value.clone())
            .map_err(|e| ConfigError::type_error(
                e.to_string(),
                std::any::type_name::<T>(),
                "JSON value"
            ))
    }
    
    /// Get configuration value by path with default
    pub async fn get_path_or<T>(&self, path: &str, default: T) -> T
    where
        T: DeserializeOwned + Clone,
    {
        self.get_path(path).await.unwrap_or(default)
    }
    
    /// Check if configuration has a path
    pub async fn has_path(&self, path: &str) -> bool {
        let data = self.data.read().await;
        self.get_nested_value(&data, path).is_ok()
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
            serde_json::Value::Object(obj) => {
                Ok(obj.keys().cloned().collect())
            }
            _ => Err(ConfigError::type_error(
                "Path does not point to an object",
                "Object",
                value.to_string()
            )),
        }
    }
    
    /// Reload configuration from all sources
    pub async fn reload(&self) -> ConfigResult<()> {
        let mut merged_data = serde_json::Value::Object(serde_json::Map::new());
        
        // Load from all sources in priority order (reverse order)
        let mut sources = self.sources.clone();
        sources.sort_by_key(|s| std::cmp::Reverse(s.priority()));
        
        for source in &sources {
            match self.loader.load(source).await {
                Ok(data) => {
                    // Update metadata
                    if let Ok(metadata) = self.loader.metadata(source).await {
                        self.metadata.insert(source.clone(), metadata);
                    }
                    
                    // Merge data
                    self.merge_values(&mut merged_data, data)?;
                }
                Err(e) => {
                    tracing::warn!("Failed to load from source {}: {}", source, e);
                }
            }
        }
        
        // Validate if validator is present
        if let Some(validator) = &self.validator {
            validator.validate(&merged_data).await?;
        }
        
        // Update configuration data
        let mut data = self.data.write().await;
        *data = merged_data;
        
        Ok(())
    }
    
    /// Get source metadata
    pub fn get_metadata(&self, source: &ConfigSource) -> Option<SourceMetadata> {
        self.metadata.get(source).map(|entry| entry.clone())
    }
    
    /// Get all source metadata
    pub fn get_all_metadata(&self) -> HashMap<ConfigSource, SourceMetadata> {
        self.metadata.iter().map(|entry| (entry.key().clone(), entry.value().clone())).collect()
    }
    
    /// Start watching for configuration changes (if hot reload is enabled)
    pub async fn start_watching(&self) -> ConfigResult<()> {
        if !self.hot_reload {
            return Ok(());
        }
        
        if let Some(watcher) = &self.watcher {
            watcher.start_watching(&self.sources).await?;
        }
        
        Ok(())
    }
    
    /// Stop watching for configuration changes
    pub async fn stop_watching(&self) -> ConfigResult<()> {
        if let Some(watcher) = &self.watcher {
            watcher.stop_watching().await?;
        }
        
        Ok(())
    }
    
    /// Get nested value from JSON using dot notation
    fn get_nested_value(&self, value: &serde_json::Value, path: &str) -> ConfigResult<&serde_json::Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = value;
        
        for part in parts {
            match current {
                serde_json::Value::Object(obj) => {
                    current = obj.get(part).ok_or_else(|| {
                        ConfigError::path_error(
                            format!("Key '{}' not found", part),
                            path.to_string()
                        )
                    })?;
                }
                serde_json::Value::Array(arr) => {
                    let index: usize = part.parse().map_err(|_| {
                        ConfigError::path_error(
                            format!("Invalid array index '{}'", part),
                            path.to_string()
                        )
                    })?;
                    current = arr.get(index).ok_or_else(|| {
                        ConfigError::path_error(
                            format!("Array index {} out of bounds", index),
                            path.to_string()
                        )
                    })?;
                }
                _ => {
                    return Err(ConfigError::path_error(
                        format!("Cannot index into {}", current),
                        path.to_string()
                    ));
                }
            }
        }
        
        Ok(current)
    }
    
    /// Merge two JSON values
    fn merge_values(&self, target: &mut serde_json::Value, source: serde_json::Value) -> ConfigResult<()> {
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

/// Configuration builder
#[derive(Debug)]
pub struct ConfigBuilder {
    /// Configuration sources
    sources: Vec<ConfigSource>,
    
    /// Default values
    defaults: Option<serde_json::Value>,
    
    /// Configuration loader
    loader: Option<Arc<dyn ConfigLoader>>,
    
    /// Configuration validator
    validator: Option<Arc<dyn ConfigValidator>>,
    
    /// Configuration watcher
    watcher: Option<Arc<dyn ConfigWatcher>>,
    
    /// Hot reload enabled
    hot_reload: bool,
    
    /// Auto-reload interval
    auto_reload_interval: Option<std::time::Duration>,
}

impl ConfigBuilder {
    /// Create a new configuration builder
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            defaults: None,
            loader: None,
            validator: None,
            watcher: None,
            hot_reload: false,
            auto_reload_interval: None,
        }
    }
    
    /// Add a configuration source
    pub fn with_source(mut self, source: ConfigSource) -> Self {
        self.sources.push(source);
        self
    }
    
    /// Add multiple configuration sources
    pub fn with_sources(mut self, sources: Vec<ConfigSource>) -> Self {
        self.sources.extend(sources);
        self
    }
    
    /// Set default values
    pub fn with_defaults<T>(mut self, defaults: T) -> ConfigResult<Self>
    where
        T: serde::Serialize,
    {
        self.defaults = Some(serde_json::to_value(defaults)?);
        Ok(self)
    }
    
    /// Set configuration loader
    pub fn with_loader(mut self, loader: Arc<dyn ConfigLoader>) -> Self {
        self.loader = Some(loader);
        self
    }
    
    /// Set configuration validator
    pub fn with_validator(mut self, validator: Arc<dyn ConfigValidator>) -> Self {
        self.validator = Some(validator);
        self
    }
    
    /// Set configuration watcher
    pub fn with_watcher(mut self, watcher: Arc<dyn ConfigWatcher>) -> Self {
        self.watcher = Some(watcher);
        self
    }
    
    /// Enable hot reload
    pub fn with_hot_reload(mut self, enabled: bool) -> Self {
        self.hot_reload = enabled;
        self
    }
    
    /// Set auto-reload interval
    pub fn with_auto_reload_interval(mut self, interval: std::time::Duration) -> Self {
        self.auto_reload_interval = Some(interval);
        self
    }
    
    /// Build the configuration
    pub async fn build(self) -> ConfigResult<Config> {
        // Use default loader if none provided
        let loader = self.loader.unwrap_or_else(|| Arc::new(CompositeLoader::default()));
        
        // Add default source if defaults are provided
        let mut sources = self.sources;
        if self.defaults.is_some() {
            sources.push(ConfigSource::Default);
        }
        
        // Sort sources by priority (higher priority first for loading)
        sources.sort_by_key(|s| s.priority());
        
        // Create configuration
        let config = Config {
            data: Arc::new(RwLock::new(serde_json::Value::Object(serde_json::Map::new()))),
            sources: sources.clone(),
            metadata: Arc::new(DashMap::new()),
            loader,
            validator: self.validator,
            watcher: self.watcher,
            hot_reload: self.hot_reload,
        };
        
        // Load initial configuration
        let mut merged_data = serde_json::Value::Object(serde_json::Map::new());
        
        // Add defaults first if present
        if let Some(defaults) = self.defaults {
            config.merge_values(&mut merged_data, defaults)?;
        }
        
        // Load from all sources
        for source in &sources {
            if matches!(source, ConfigSource::Default) {
                continue; // Already handled defaults
            }
            
            match config.loader.load(source).await {
                Ok(data) => {
                    // Update metadata
                    if let Ok(metadata) = config.loader.metadata(source).await {
                        config.metadata.insert(source.clone(), metadata);
                    }
                    
                    // Merge data
                    config.merge_values(&mut merged_data, data)?;
                }
                Err(e) => {
                    tracing::warn!("Failed to load from source {}: {}", source, e);
                }
            }
        }
        
        // Validate if validator is present
        if let Some(validator) = &config.validator {
            validator.validate(&merged_data).await?;
        }
        
        // Set initial data
        {
            let mut data = config.data.write().await;
            *data = merged_data;
        }
        
        // Start watching if hot reload is enabled
        if config.hot_reload {
            config.start_watching().await?;
        }
        
        // Start auto-reload if interval is set
        if let Some(interval) = self.auto_reload_interval {
            let config_clone = Arc::new(config);
            let config_weak = Arc::downgrade(&config_clone);
            
            tokio::spawn(async move {
                let mut interval_timer = tokio::time::interval(interval);
                
                loop {
                    interval_timer.tick().await;
                    
                    if let Some(config) = config_weak.upgrade() {
                        if let Err(e) = config.reload().await {
                            tracing::error!("Auto-reload failed: {}", e);
                        }
                    } else {
                        break; // Config has been dropped
                    }
                }
            });
            
            return Ok(Arc::try_unwrap(config_clone).unwrap_or_else(|arc| {
                // This should not happen in normal circumstances
                panic!("Failed to unwrap Arc<Config>");
            }));
        }
        
        Ok(config)
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
