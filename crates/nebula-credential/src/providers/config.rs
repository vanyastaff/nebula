//! Provider configuration trait and error types

/// Configuration error types
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Invalid configuration value
    #[error("Invalid configuration: {field}: {reason}")]
    InvalidValue { field: String, reason: String },

    /// Missing required configuration
    #[error("Missing required configuration: {field}")]
    MissingRequired { field: String },

    /// Configuration validation failed
    #[error("Validation failed: {0}")]
    ValidationFailed(String),
}

/// Trait for storage provider configuration
///
/// All provider configs must implement this trait to ensure
/// validation before initialization.
///
/// # Contract
///
/// - `validate()` must check all configuration parameters and return errors with
///   actionable messages indicating what is wrong and how to fix it
/// - `provider_name()` must return a static string identifying the provider
///   for logging and metrics
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::providers::ProviderConfig;
///
/// impl ProviderConfig for MyProviderConfig {
///     fn validate(&self) -> Result<(), ConfigError> {
///         if self.timeout.as_secs() < 1 || self.timeout.as_secs() > 60 {
///             return Err(ConfigError::InvalidValue {
///                 field: "timeout".into(),
///                 reason: "must be between 1 and 60 seconds".into(),
///             });
///         }
///         Ok(())
///     }
///
///     fn provider_name(&self) -> &'static str {
///         "MyProvider"
///     }
/// }
/// ```
pub trait ProviderConfig: Send + Sync + Clone {
    /// Validate configuration parameters
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Configuration is valid
    /// * `Err(ConfigError)` - Configuration has errors with details
    fn validate(&self) -> Result<(), ConfigError>;

    /// Get provider name for logging and metrics
    ///
    /// # Returns
    ///
    /// Static string identifying this provider (e.g., "LocalStorage", "AWS", "Azure")
    fn provider_name(&self) -> &'static str;
}
