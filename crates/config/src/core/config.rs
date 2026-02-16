//! Main configuration container

use super::{ConfigError, ConfigResult, ConfigSource, SourceMetadata};
use super::{ConfigLoader, ConfigValidator, ConfigWatcher};
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

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

    /// Cancellation token for background tasks (auto-reload, etc.)
    cancel_token: CancellationToken,
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
            cancel_token: CancellationToken::new(),
        }
    }

    /// Get the cancellation token for background tasks
    pub(crate) fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }

    /// Get entire configuration as typed value
    pub async fn get_all<T>(&self) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        T::deserialize(&*data).map_err(|e| {
            ConfigError::type_error(e.to_string(), std::any::type_name::<T>(), "JSON value")
        })
    }

    /// Get configuration value by path
    pub async fn get<T>(&self, path: &str) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        let value = self.get_nested_value(&data, path)?;
        T::deserialize(value).map_err(|e| {
            ConfigError::type_error(e.to_string(), std::any::type_name::<T>(), "JSON value")
        })
    }

    /// Get configuration value by path (alias for get)
    #[deprecated(since = "0.2.0", note = "use `get` instead")]
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
    ///
    /// Sources are loaded **concurrently** for maximum throughput,
    /// then merged in priority order (pre-sorted at construction time).
    pub async fn reload(&self) -> ConfigResult<()> {
        nebula_log::info!(
            "Reloading configuration from {} sources",
            self.sources.len()
        );

        // Load all sources concurrently
        let loader = &self.loader;
        let load_futures = self.sources.iter().map(|source| async move {
            let data = loader.load(source).await;
            let metadata = loader.metadata(source).await.ok();
            (source, data, metadata)
        });
        let results = futures::future::join_all(load_futures).await;

        // Merge in priority order (sources are pre-sorted at construction time)
        let mut merged_data = serde_json::Value::Object(serde_json::Map::new());
        for (source, result, metadata) in results {
            match result {
                Ok(data) => {
                    nebula_log::debug!("Loaded configuration from source: {}", source);

                    if let Some(metadata) = metadata {
                        self.metadata.insert(source.clone(), metadata);
                    }

                    merge_json(&mut merged_data, data)?;
                }
                Err(e) => {
                    nebula_log::warn!("Failed to load from source {}: {}", source, e);

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
        self.metadata.get(source).map(|entry| entry.value().clone())
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
        // Cancel background tasks (auto-reload, etc.)
        self.cancel_token.cancel();

        if let Some(watcher) = &self.watcher {
            nebula_log::info!("Stopping configuration watcher");
            watcher.stop_watching().await?;
        }

        Ok(())
    }

    /// Check if watching for changes
    pub fn is_watching(&self) -> bool {
        self.watcher.as_ref().is_some_and(|w| w.is_watching())
    }

    /// Get nested value from JSON using dot notation (zero-alloc path traversal)
    fn get_nested_value<'a>(
        &self,
        value: &'a serde_json::Value,
        path: &str,
    ) -> ConfigResult<&'a serde_json::Value> {
        if path.is_empty() {
            return Ok(value);
        }

        let mut current = value;

        for part in path.split('.') {
            match current {
                serde_json::Value::Object(obj) => {
                    current = obj.get(part).ok_or_else(|| {
                        ConfigError::path_error(format!("Key '{part}' not found"), path.to_string())
                    })?;
                }
                serde_json::Value::Array(arr) => {
                    let index: usize = part.parse().map_err(|_| {
                        ConfigError::path_error(
                            format!("Invalid array index '{part}'"),
                            path.to_string(),
                        )
                    })?;
                    current = arr.get(index).ok_or_else(|| {
                        ConfigError::path_error(
                            format!("Array index {index} out of bounds (size: {})", arr.len()),
                            path.to_string(),
                        )
                    })?;
                }
                _ => {
                    return Err(ConfigError::path_error(
                        format!(
                            "Cannot index into {} with '{part}'",
                            json_type_name(current),
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

        // Split into parent path and final key to avoid Vec allocation
        let (parent_path, final_key) = match path.rsplit_once('.') {
            Some((parent, key)) => (Some(parent), key),
            None => (None, path),
        };

        // Navigate to the parent
        let current = if let Some(parent_path) = parent_path {
            let mut current = &mut *value;
            for part in parent_path.split('.') {
                match current {
                    serde_json::Value::Object(obj) => {
                        current = obj
                            .entry(part.to_string())
                            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                    }
                    serde_json::Value::Array(arr) => {
                        let index: usize = part.parse().map_err(|_| {
                            ConfigError::path_error(
                                format!("Invalid array index '{part}'"),
                                path.to_string(),
                            )
                        })?;
                        while arr.len() <= index {
                            arr.push(serde_json::Value::Null);
                        }
                        current = &mut arr[index];
                    }
                    _ => {
                        return Err(ConfigError::path_error(
                            format!("Cannot navigate into {} type", json_type_name(current)),
                            path.to_string(),
                        ));
                    }
                }
            }
            current
        } else {
            value
        };

        // Set the final value
        match current {
            serde_json::Value::Object(obj) => {
                obj.insert(final_key.to_string(), new_value);
            }
            serde_json::Value::Array(arr) => {
                let index: usize = final_key.parse().map_err(|_| {
                    ConfigError::path_error(
                        format!("Invalid array index '{final_key}'"),
                        path.to_string(),
                    )
                })?;
                while arr.len() <= index {
                    arr.push(serde_json::Value::Null);
                }
                arr[index] = new_value;
            }
            _ => {
                return Err(ConfigError::path_error(
                    format!("Cannot set value in {} type", json_type_name(current)),
                    path.to_string(),
                ));
            }
        }

        Ok(())
    }

    // ==================== Dynamic Value Integration ====================

    /// Get entire configuration as dynamic value
    pub async fn as_value(&self) -> serde_json::Value {
        let data = self.data.read().await;
        data.clone()
    }

    /// Get configuration value by path as dynamic value
    pub async fn get_value(&self, path: &str) -> ConfigResult<serde_json::Value> {
        let data = self.data.read().await;
        let json_value = self.get_nested_value(&data, path)?;
        Ok(json_value.clone())
    }

    /// Set configuration value by path
    pub async fn set_value(&self, path: &str, value: serde_json::Value) -> ConfigResult<()> {
        let mut data = self.data.write().await;
        self.set_nested_value(&mut data, path, value)?;
        Ok(())
    }

    /// Set typed configuration with automatic serialization
    pub async fn set_typed<T>(&self, path: &str, value: T) -> ConfigResult<()>
    where
        T: serde::Serialize,
    {
        let json_value = serde_json::to_value(value).map_err(|e| {
            ConfigError::type_error(
                format!("Failed to serialize: {e}"),
                "JSON value",
                std::any::type_name::<T>(),
            )
        })?;
        self.set_value(path, json_value).await
    }

    /// Get all configuration as flat key-value map
    pub async fn flatten(&self) -> HashMap<String, serde_json::Value> {
        let value = self.as_value().await;
        let mut map = HashMap::new();
        flatten_into("", &value, &mut map);
        map
    }

    /// Merge configuration from dynamic value
    pub async fn merge(&self, value: serde_json::Value) -> ConfigResult<()> {
        let mut data = self.data.write().await;
        merge_json(&mut data, value)
    }
}

/// Get human-readable type name for a JSON value (zero-alloc)
pub(crate) fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Merge two JSON values (free function, no allocations beyond the merge itself)
pub(crate) fn merge_json(
    target: &mut serde_json::Value,
    source: serde_json::Value,
) -> ConfigResult<()> {
    match (target, source) {
        (serde_json::Value::Object(target_obj), serde_json::Value::Object(source_obj)) => {
            for (key, value) in source_obj {
                if let Some(existing) = target_obj.get_mut(&key) {
                    merge_json(existing, value)?;
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

/// Flatten serde_json::Value into a map with dot-notation keys
fn flatten_into(
    prefix: &str,
    value: &serde_json::Value,
    map: &mut HashMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let full_key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_into(&full_key, val, map);
            }
        }
        serde_json::Value::Array(arr) => {
            for (index, val) in arr.iter().enumerate() {
                let full_key = if prefix.is_empty() {
                    index.to_string()
                } else {
                    format!("{prefix}[{index}]")
                };
                flatten_into(&full_key, val, map);
            }
        }
        _ => {
            map.insert(prefix.to_string(), value.clone());
        }
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("sources", &self.sources.len())
            .field("hot_reload", &self.hot_reload)
            .field("watching", &self.is_watching())
            .field("has_validator", &self.validator.is_some())
            .field("has_watcher", &self.watcher.is_some())
            .finish()
    }
}

// Cleanup on drop
impl Drop for Config {
    fn drop(&mut self) {
        // Cancel all background tasks (auto-reload, etc.)
        self.cancel_token.cancel();

        if let Some(watcher) = &self.watcher
            && watcher.is_watching()
        {
            nebula_log::debug!("Config dropped while still watching");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_config(data: serde_json::Value) -> Config {
        Config::new(
            data,
            vec![ConfigSource::Default],
            Arc::new(crate::loaders::CompositeLoader::default()),
            None,
            None,
            false,
        )
    }

    #[tokio::test]
    async fn test_get_and_get_all() {
        let cfg = test_config(json!({
            "name": "app",
            "port": 8080,
            "nested": {"key": "value"}
        }));

        let name: String = cfg.get("name").await.unwrap();
        assert_eq!(name, "app");

        let port: u16 = cfg.get("port").await.unwrap();
        assert_eq!(port, 8080);

        let nested_val: String = cfg.get("nested.key").await.unwrap();
        assert_eq!(nested_val, "value");

        // get_all deserializes entire config
        #[derive(serde::Deserialize)]
        struct AppConfig {
            name: String,
            port: u16,
        }
        let all: AppConfig = cfg.get_all().await.unwrap();
        assert_eq!(all.name, "app");
        assert_eq!(all.port, 8080);
    }

    #[tokio::test]
    async fn test_get_or_and_get_or_else() {
        let cfg = test_config(json!({"existing": "hello"}));

        let val: String = cfg.get_or("existing", "default".to_string()).await;
        assert_eq!(val, "hello");

        let val: String = cfg.get_or("missing", "default".to_string()).await;
        assert_eq!(val, "default");

        let val: String = cfg.get_or_else("missing", || "computed".to_string()).await;
        assert_eq!(val, "computed");
    }

    #[tokio::test]
    async fn test_has_and_get_opt() {
        let cfg = test_config(json!({"key": "value", "nested": {"a": 1}}));

        assert!(cfg.has("key").await);
        assert!(cfg.has("nested.a").await);
        assert!(!cfg.has("missing").await);
        assert!(!cfg.has("nested.b").await);

        let some: Option<String> = cfg.get_opt("key").await;
        assert_eq!(some, Some("value".to_string()));

        let none: Option<String> = cfg.get_opt("missing").await;
        assert_eq!(none, None);
    }

    #[tokio::test]
    async fn test_keys() {
        let cfg = test_config(json!({
            "a": 1,
            "b": 2,
            "nested": {"x": 10, "y": 20}
        }));

        let mut root_keys = cfg.keys(None).await.unwrap();
        root_keys.sort();
        assert_eq!(root_keys, vec!["a", "b", "nested"]);

        let mut nested_keys = cfg.keys(Some("nested")).await.unwrap();
        nested_keys.sort();
        assert_eq!(nested_keys, vec!["x", "y"]);

        // Non-object path errors
        assert!(cfg.keys(Some("a")).await.is_err());
    }

    #[tokio::test]
    async fn test_get_raw_and_get_value() {
        let data = json!({"key": "value", "num": 42});
        let cfg = test_config(data.clone());

        let raw_all = cfg.get_raw(None).await.unwrap();
        assert_eq!(raw_all, data);

        let raw_key = cfg.get_raw(Some("key")).await.unwrap();
        assert_eq!(raw_key, json!("value"));

        let val = cfg.get_value("num").await.unwrap();
        assert_eq!(val, json!(42));
    }

    #[tokio::test]
    async fn test_as_value() {
        let data = json!({"hello": "world"});
        let cfg = test_config(data.clone());
        assert_eq!(cfg.as_value().await, data);
    }

    #[tokio::test]
    async fn test_set_value_and_set_typed() {
        let cfg = test_config(json!({"a": 1}));

        cfg.set_value("b", json!("new")).await.unwrap();
        let val: String = cfg.get("b").await.unwrap();
        assert_eq!(val, "new");

        // Set nested path (creates intermediary objects)
        cfg.set_value("nested.deep", json!(true)).await.unwrap();
        let val: bool = cfg.get("nested.deep").await.unwrap();
        assert!(val);

        // set_typed serializes automatically
        cfg.set_typed("count", 42u32).await.unwrap();
        let val: u32 = cfg.get("count").await.unwrap();
        assert_eq!(val, 42);
    }

    #[tokio::test]
    async fn test_flatten() {
        let cfg = test_config(json!({
            "server": {"host": "localhost", "port": 8080},
            "tags": ["a", "b"]
        }));

        let flat = cfg.flatten().await;
        assert_eq!(flat["server.host"], json!("localhost"));
        assert_eq!(flat["server.port"], json!(8080));
        assert_eq!(flat["tags[0]"], json!("a"));
        assert_eq!(flat["tags[1]"], json!("b"));
    }

    #[tokio::test]
    async fn test_merge() {
        let cfg = test_config(json!({
            "a": 1,
            "nested": {"x": 10, "y": 20}
        }));

        cfg.merge(json!({
            "b": 2,
            "nested": {"y": 99, "z": 30}
        }))
        .await
        .unwrap();

        let val: i64 = cfg.get("a").await.unwrap();
        assert_eq!(val, 1); // preserved
        let val: i64 = cfg.get("b").await.unwrap();
        assert_eq!(val, 2); // added
        let val: i64 = cfg.get("nested.x").await.unwrap();
        assert_eq!(val, 10); // preserved
        let val: i64 = cfg.get("nested.y").await.unwrap();
        assert_eq!(val, 99); // overwritten
        let val: i64 = cfg.get("nested.z").await.unwrap();
        assert_eq!(val, 30); // added
    }

    #[test]
    fn test_json_type_name() {
        assert_eq!(json_type_name(&json!(null)), "null");
        assert_eq!(json_type_name(&json!(true)), "boolean");
        assert_eq!(json_type_name(&json!(42)), "number");
        assert_eq!(json_type_name(&json!("hi")), "string");
        assert_eq!(json_type_name(&json!([1, 2])), "array");
        assert_eq!(json_type_name(&json!({"a": 1})), "object");
    }

    #[test]
    fn test_merge_json() {
        let mut target = json!({"a": 1, "nested": {"x": 10}});
        merge_json(&mut target, json!({"b": 2, "nested": {"y": 20}})).unwrap();
        assert_eq!(
            target,
            json!({"a": 1, "b": 2, "nested": {"x": 10, "y": 20}})
        );

        // Scalar overwrite
        let mut target2 = json!({"key": "old"});
        merge_json(&mut target2, json!({"key": "new"})).unwrap();
        assert_eq!(target2["key"], json!("new"));
    }

    #[test]
    fn test_config_debug() {
        let cfg = test_config(json!({}));
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("Config"));
        assert!(debug.contains("sources"));
        assert!(debug.contains("hot_reload"));
        assert!(debug.contains("watching"));
        assert!(debug.contains("has_validator"));
        assert!(debug.contains("has_watcher"));
    }
}
