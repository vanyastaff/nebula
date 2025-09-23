//! Configuration builder

use super::{Config, ConfigError, ConfigResult, ConfigSource};
use crate::loaders::CompositeLoader;
use std::sync::Arc;
use crate::{ConfigLoader, ConfigValidator, ConfigWatcher};

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

    /// Set default values from JSON
    pub fn with_defaults_json(mut self, defaults: serde_json::Value) -> Self {
        self.defaults = Some(defaults);
        self
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

    /// Set whether to fail on missing optional sources
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

        // Sort sources by priority (higher priority first for loading)
        sources.sort_by_key(|s| s.priority());

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

        // Load from all sources
        for source in &sources {
            if matches!(source, ConfigSource::Default) {
                continue; // Already handled defaults
            }

            match loader.load(source).await {
                Ok(data) => {
                    nebula_log::debug!(
                        action = "source_loaded",
                        source = %source,
                        data_keys = data.as_object().map(|o| o.len()).unwrap_or(0),
                        "Successfully loaded configuration from source"
                    );

                    // Create temporary config for merging
                    let temp_config = Config::new(
                        serde_json::Value::Object(serde_json::Map::new()),
                        vec![],
                        Arc::clone(&loader),
                        None,
                        None,
                        false,
                    );

                    temp_config.merge_values(&mut merged_data, data)?;
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
            Self::start_auto_reload(Arc::new(config.clone()), interval).await;
        }

        Ok(config)
    }

    /// Start auto-reload task
    async fn start_auto_reload(config: Arc<Config>, interval: std::time::Duration) {
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval_timer.tick().await;

                if let Err(e) = config.reload().await {
                    nebula_log::error!("Auto-reload failed: {}", e);
                }
            }
        });
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