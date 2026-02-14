//! Result type and utilities for configuration operations

use super::error::ConfigError;
use std::future::Future;

/// Standard result type for configuration operations
pub type ConfigResult<T> = Result<T, ConfigError>;

/// Extension trait for Result types to add configuration-specific utilities
pub trait ConfigResultExt<T> {
    /// Convert error with additional context
    fn with_context<F>(self, f: F) -> ConfigResult<T>
    where
        F: FnOnce() -> String;

    /// Map error to a different ConfigError variant
    fn map_config_error<F>(self, f: F) -> ConfigResult<T>
    where
        F: FnOnce(ConfigError) -> ConfigError;

    /// Convert to option, logging error if present
    fn log_error(self) -> Option<T>;

    /// Convert to option with custom error handler
    fn handle_error<F>(self, f: F) -> Option<T>
    where
        F: FnOnce(&ConfigError);
}

impl<T> ConfigResultExt<T> for ConfigResult<T> {
    fn with_context<F>(self, f: F) -> ConfigResult<T>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|e| match e {
            ConfigError::SourceError { message, origin } => {
                let ctx = f();
                ConfigError::SourceError {
                    message: format!("{ctx}: {message}"),
                    origin,
                }
            }
            other => ConfigError::SourceError {
                message: f(),
                origin: other.to_string(),
            },
        })
    }

    fn map_config_error<F>(self, f: F) -> ConfigResult<T>
    where
        F: FnOnce(ConfigError) -> ConfigError,
    {
        self.map_err(f)
    }

    fn log_error(self) -> Option<T> {
        match self {
            Ok(value) => Some(value),
            Err(e) => {
                nebula_log::error!("Configuration error: {}", e);
                None
            }
        }
    }

    fn handle_error<F>(self, f: F) -> Option<T>
    where
        F: FnOnce(&ConfigError),
    {
        match self {
            Ok(value) => Some(value),
            Err(e) => {
                f(&e);
                None
            }
        }
    }
}

/// Helper for aggregating multiple results
pub struct ConfigResultAggregator {
    errors: Vec<ConfigError>,
    context: Option<String>,
}

impl ConfigResultAggregator {
    /// Create a new aggregator
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            context: None,
        }
    }

    /// Create with context
    pub fn with_context(context: impl Into<String>) -> Self {
        Self {
            errors: Vec::new(),
            context: Some(context.into()),
        }
    }

    /// Add a result, returning value if successful
    pub fn add<T>(&mut self, result: ConfigResult<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.errors.push(e);
                None
            }
        }
    }

    /// Check if any result was successful
    pub fn check(&mut self, result: ConfigResult<()>) -> bool {
        match result {
            Ok(()) => true,
            Err(e) => {
                self.errors.push(e);
                false
            }
        }
    }

    /// Check if there were any errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Get error count
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Get all errors
    pub fn errors(&self) -> &[ConfigError] {
        &self.errors
    }

    /// Finish aggregation and return result
    pub fn finish(self) -> ConfigResult<()> {
        if self.errors.is_empty() {
            Ok(())
        } else if self.errors.len() == 1 {
            Err(self.errors.into_iter().next().expect("len checked above"))
        } else {
            let message = if let Some(ctx) = self.context {
                format!(
                    "{}: {} errors occurred:\n{}",
                    ctx,
                    self.errors.len(),
                    self.errors
                        .iter()
                        .enumerate()
                        .map(|(i, e)| format!("  {}. {e}", i + 1))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            } else {
                format!(
                    "Multiple errors occurred ({}):\n{}",
                    self.errors.len(),
                    self.errors
                        .iter()
                        .enumerate()
                        .map(|(i, e)| format!("  {}. {e}", i + 1))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };

            Err(ConfigError::ValidationError {
                message,
                field: None,
            })
        }
    }
}

impl Default for ConfigResultAggregator {
    fn default() -> Self {
        Self::new()
    }
}

/// Try multiple sources until one succeeds
pub async fn try_sources<F, T, Fut>(sources: &[super::ConfigSource], mut f: F) -> ConfigResult<T>
where
    F: FnMut(&super::ConfigSource) -> Fut,
    Fut: Future<Output = ConfigResult<T>>,
{
    let mut last_error = None;

    for source in sources {
        nebula_log::trace!("Trying source: {}", source);
        match f(source).await {
            Ok(result) => {
                nebula_log::debug!("Successfully loaded from source: {}", source);
                return Ok(result);
            }
            Err(e) => {
                nebula_log::debug!("Failed to load from source {}: {}", source, e);
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| ConfigError::SourceError {
        message: "No sources provided".to_string(),
        origin: "try_sources".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_aggregator() {
        let mut aggregator = ConfigResultAggregator::new();

        aggregator.add(Ok::<_, ConfigError>(42));
        aggregator.add(Err::<i32, ConfigError>(ConfigError::validation_error(
            "Test error 1",
            None,
        )));
        aggregator.add(Err::<i32, ConfigError>(ConfigError::validation_error(
            "Test error 2",
            None,
        )));

        assert!(aggregator.has_errors());
        assert_eq!(aggregator.error_count(), 2);

        let result = aggregator.finish();
        assert!(result.is_err());
    }

    #[test]
    fn test_result_ext() {
        let result: ConfigResult<i32> = Err(ConfigError::validation_error("test", None));
        let with_context = result.with_context(|| "Additional context".to_string());
        assert!(with_context.is_err());
    }
}
