//! Configuration builder

use super::config::merge_json;
use super::{Config, ConfigError, ConfigResult, ConfigSource};
use crate::loaders::CompositeLoader;
use crate::{ConfigLoader, ConfigValidator, ConfigWatcher};
use std::sync::Arc;

/// Configuration builder
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

    /// Whether to fail on missing optional sources
    fail_on_missing: bool,
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
            fail_on_missing: false,
        }
    }

    /// Add a configuration source
    #[must_use = "builder methods must be chained or built"]
    pub fn with_source(mut self, source: ConfigSource) -> Self {
        self.sources.push(source);
        self
    }

    /// Add multiple configuration sources
    #[must_use = "builder methods must be chained or built"]
    pub fn with_sources(mut self, sources: Vec<ConfigSource>) -> Self {
        self.sources.extend(sources);
        self
    }

    /// Set default values
    #[must_use = "builder methods must be chained or built"]
    pub fn with_defaults<T>(mut self, defaults: T) -> ConfigResult<Self>
    where
        T: serde::Serialize,
    {
        self.defaults = Some(serde_json::to_value(defaults)?);
        Ok(self)
    }

    /// Set default values from JSON
    #[must_use = "builder methods must be chained or built"]
    pub fn with_defaults_json(mut self, defaults: serde_json::Value) -> Self {
        self.defaults = Some(defaults);
        self
    }

    /// Set configuration loader
    #[must_use = "builder methods must be chained or built"]
    pub fn with_loader(mut self, loader: Arc<dyn ConfigLoader>) -> Self {
        self.loader = Some(loader);
        self
    }

    /// Set configuration validator
    #[must_use = "builder methods must be chained or built"]
    pub fn with_validator(mut self, validator: Arc<dyn ConfigValidator>) -> Self {
        self.validator = Some(validator);
        self
    }

    /// Set configuration watcher
    #[must_use = "builder methods must be chained or built"]
    pub fn with_watcher(mut self, watcher: Arc<dyn ConfigWatcher>) -> Self {
        self.watcher = Some(watcher);
        self
    }

    /// Enable hot reload
    #[must_use = "builder methods must be chained or built"]
    pub fn with_hot_reload(mut self, enabled: bool) -> Self {
        self.hot_reload = enabled;
        self
    }

    /// Set auto-reload interval
    #[must_use = "builder methods must be chained or built"]
    pub fn with_auto_reload_interval(mut self, interval: std::time::Duration) -> Self {
        self.auto_reload_interval = Some(interval);
        self
    }

    /// Set whether to fail on missing optional sources
    #[must_use = "builder methods must be chained or built"]
    pub fn with_fail_on_missing(mut self, fail: bool) -> Self {
        self.fail_on_missing = fail;
        self
    }

    /// Validate builder configuration
    fn validate(&self) -> ConfigResult<()> {
        // Ensure at least one source if no defaults
        if self.sources.is_empty() && self.defaults.is_none() {
            return Err(ConfigError::validation_error(
                "No configuration sources or defaults provided",
                None,
            ));
        }

        Ok(())
    }

    /// Build the configuration
    pub async fn build(self) -> ConfigResult<Config> {
        // Validate builder
        self.validate()?;

        // Use default loader if none provided
        let loader = self
            .loader
            .unwrap_or_else(|| Arc::new(CompositeLoader::default()));

        // Add default source if defaults are provided
        let mut sources = self.sources;
        if self.defaults.is_some() {
            sources.insert(0, ConfigSource::Default);
        }

        // Sort sources by priority (higher number = lower priority, loaded first so higher overrides)
        sources.sort_by_key(|s| std::cmp::Reverse(s.priority()));

        // Load initial configuration
        let mut merged_data = serde_json::Value::Object(serde_json::Map::new());

        // Add defaults first if present
        if let Some(defaults) = self.defaults {
            nebula_log::debug!(
                action = "applying_defaults",
                default_keys = defaults.as_object().map(|o| o.len()).unwrap_or(0),
                "Applying default configuration"
            );
            merged_data = defaults;
        }

        // Load all sources concurrently, then merge in priority order
        let loadable: Vec<_> = sources
            .iter()
            .filter(|s| !matches!(s, ConfigSource::Default))
            .collect();
        let load_results =
            futures::future::join_all(loadable.iter().map(|source| loader.load(source))).await;

        for (source, result) in loadable.iter().zip(load_results) {
            match result {
                Ok(data) => {
                    nebula_log::debug!(
                        action = "source_loaded",
                        source = %source,
                        data_keys = data.as_object().map(|o| o.len()).unwrap_or(0),
                        "Successfully loaded configuration from source"
                    );

                    merge_json(&mut merged_data, data)?;
                }
                Err(e) => {
                    if self.fail_on_missing || !source.is_optional() {
                        return Err(e);
                    }
                    nebula_log::warn!(
                        action = "source_load_failed",
                        source = %source,
                        error = %e,
                        optional = source.is_optional(),
                        "Failed to load optional configuration source"
                    );
                }
            }
        }

        // Validate if validator is present
        if let Some(ref validator) = self.validator {
            nebula_log::debug!("Validating initial configuration");
            validator.validate(&merged_data).await?;
        }

        // Create configuration
        let config = Config::new(
            merged_data,
            sources,
            loader,
            self.validator,
            self.watcher,
            self.hot_reload,
        );

        // Start watching if hot reload is enabled
        if self.hot_reload {
            config.start_watching().await?;
        }

        // Start auto-reload if interval is set
        if let Some(interval) = self.auto_reload_interval {
            let token = config.cancel_token().clone();
            let config_arc = Arc::new(config.clone());
            tokio::spawn(async move {
                let mut interval_timer = tokio::time::interval(interval);
                interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                loop {
                    tokio::select! {
                        _ = token.cancelled() => {
                            nebula_log::debug!("Auto-reload task cancelled");
                            break;
                        }
                        _ = interval_timer.tick() => {
                            if let Err(e) = config_arc.reload().await {
                                nebula_log::error!("Auto-reload failed: {}", e);
                            }
                        }
                    }
                }
            });
        }

        Ok(config)
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ConfigBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigBuilder")
            .field("sources", &self.sources.len())
            .field("has_defaults", &self.defaults.is_some())
            .field("has_loader", &self.loader.is_some())
            .field("has_validator", &self.validator.is_some())
            .field("has_watcher", &self.watcher.is_some())
            .field("hot_reload", &self.hot_reload)
            .field("auto_reload_interval", &self.auto_reload_interval)
            .field("fail_on_missing", &self.fail_on_missing)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_builder_defaults_only() {
        let config = ConfigBuilder::new()
            .with_defaults_json(json!({"name": "app", "port": 8080}))
            .build()
            .await
            .unwrap();

        let name: String = config.get("name").await.unwrap();
        assert_eq!(name, "app");
        let port: u16 = config.get("port").await.unwrap();
        assert_eq!(port, 8080);
    }

    #[tokio::test]
    async fn test_builder_with_typed_defaults() {
        #[derive(serde::Serialize)]
        struct Defaults {
            host: String,
            port: u16,
        }

        let config = ConfigBuilder::new()
            .with_defaults(Defaults {
                host: "localhost".into(),
                port: 3000,
            })
            .unwrap()
            .build()
            .await
            .unwrap();

        let host: String = config.get("host").await.unwrap();
        assert_eq!(host, "localhost");
    }

    #[tokio::test]
    async fn test_builder_no_sources_no_defaults_fails() {
        let result = ConfigBuilder::new().build().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No configuration sources"));
    }

    #[tokio::test]
    async fn test_builder_with_validator() {
        use crate::validators::FunctionValidator;

        // Valid config passes
        let config = ConfigBuilder::new()
            .with_defaults_json(json!({"name": "app"}))
            .with_validator(Arc::new(FunctionValidator::new(|data| {
                if data.get("name").is_none() {
                    Err(ConfigError::validation("name required"))
                } else {
                    Ok(())
                }
            })))
            .build()
            .await;
        assert!(config.is_ok());

        // Invalid config fails build
        let result = ConfigBuilder::new()
            .with_defaults_json(json!({"port": 8080}))
            .with_validator(Arc::new(FunctionValidator::new(|data| {
                if data.get("name").is_none() {
                    Err(ConfigError::validation("name required"))
                } else {
                    Ok(())
                }
            })))
            .build()
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_debug() {
        let builder = ConfigBuilder::new()
            .with_source(ConfigSource::Env)
            .with_hot_reload(true);
        let debug = format!("{:?}", builder);
        assert!(debug.contains("ConfigBuilder"));
        assert!(debug.contains("sources"));
        assert!(debug.contains("hot_reload"));
    }
}
