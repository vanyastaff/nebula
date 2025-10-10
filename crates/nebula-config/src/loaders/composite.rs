//! Composite loader that combines multiple loaders

// Standard library
use std::sync::Arc;

// External dependencies
use async_trait::async_trait;

// Internal crates
use crate::core::{ConfigError, ConfigLoader, ConfigResult, ConfigSource, SourceMetadata};

/// Composite configuration loader
pub struct CompositeLoader {
    /// List of loaders in priority order
    loaders: Vec<Arc<dyn ConfigLoader>>,
    /// Whether to fail fast on first error
    fail_fast: bool,
}

impl std::fmt::Debug for CompositeLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeLoader")
            .field("loaders", &format!("{} loaders", self.loaders.len()))
            .field("fail_fast", &self.fail_fast)
            .finish()
    }
}

impl CompositeLoader {
    /// Create a new composite loader
    pub fn new() -> Self {
        Self {
            loaders: Vec::new(),
            fail_fast: true,
        }
    }

    /// Set whether to fail fast on first error
    #[must_use = "builder methods must be chained or built"]
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    /// Add a loader
    #[must_use = "builder methods must be chained or built"]
    pub fn add_loader<L: ConfigLoader + 'static>(mut self, loader: L) -> Self {
        self.loaders.push(Arc::new(loader));
        self
    }

    /// Add a shared loader
    #[must_use = "builder methods must be chained or built"]
    pub fn add_shared_loader(mut self, loader: Arc<dyn ConfigLoader>) -> Self {
        self.loaders.push(loader);
        self
    }

    /// Create default composite loader with file and env loaders
    pub fn default_loaders() -> Self {
        use super::{EnvLoader, FileLoader};

        Self::new()
            .add_loader(FileLoader::new())
            .add_loader(EnvLoader::new())
    }

    /// Get the first loader that supports the source
    fn get_loader_for(&self, source: &ConfigSource) -> Option<&Arc<dyn ConfigLoader>> {
        self.loaders.iter().find(|loader| loader.supports(source))
    }

    /// Try all loaders until one succeeds
    async fn try_all_loaders(&self, source: &ConfigSource) -> ConfigResult<serde_json::Value> {
        let mut last_error = None;

        for loader in &self.loaders {
            if !loader.supports(source) {
                continue;
            }

            match loader.load(source).await {
                Ok(value) => return Ok(value),
                Err(e) => {
                    nebula_log::debug!("Loader failed for source {}: {}", source.name(), e);
                    last_error = Some(e);

                    if self.fail_fast {
                        return Err(last_error.unwrap_or_else(|| {
                            ConfigError::source_error(
                                "Loader failed without an error".to_string(),
                                source.name(),
                            )
                        }));
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            ConfigError::source_error(
                format!("No loader supports source type: {}", source.name()),
                source.name(),
            )
        }))
    }
}

impl Default for CompositeLoader {
    fn default() -> Self {
        Self::default_loaders()
    }
}

#[async_trait]
impl ConfigLoader for CompositeLoader {
    async fn load(&self, source: &ConfigSource) -> ConfigResult<serde_json::Value> {
        if self.fail_fast {
            // Use first matching loader
            if let Some(loader) = self.get_loader_for(source) {
                loader.load(source).await
            } else {
                Err(ConfigError::source_error(
                    format!("No loader supports source type: {}", source.name()),
                    source.name(),
                ))
            }
        } else {
            // Try all loaders until one succeeds
            self.try_all_loaders(source).await
        }
    }

    fn supports(&self, source: &ConfigSource) -> bool {
        self.loaders.iter().any(|loader| loader.supports(source))
    }

    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata> {
        if let Some(loader) = self.get_loader_for(source) {
            loader.metadata(source).await
        } else {
            Err(ConfigError::source_error(
                format!("No loader supports source type: {}", source.name()),
                source.name(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loaders::{EnvLoader, FileLoader};

    #[test]
    fn test_composite_loader_creation() {
        let loader = CompositeLoader::new()
            .add_loader(FileLoader::new())
            .add_loader(EnvLoader::new())
            .with_fail_fast(false);

        assert!(!loader.fail_fast);
        assert_eq!(loader.loaders.len(), 2);
    }

    #[test]
    fn test_supports_multiple_sources() {
        let loader = CompositeLoader::default_loaders();

        assert!(loader.supports(&ConfigSource::File("config.json".into())));
        assert!(loader.supports(&ConfigSource::Env));
        assert!(loader.supports(&ConfigSource::EnvWithPrefix("APP".to_string())));
        assert!(!loader.supports(&ConfigSource::Remote("http://example.com".to_string())));
    }
}
