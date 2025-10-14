//! Parameter Error Types (Standalone)
//!
//! Following the pattern used by Tokio, Bevy, and other major Rust projects.

use nebula_core::{KeyParseError, ParameterKey};
use thiserror::Error;

// ============================================================================
// MAIN ERROR TYPE
// ============================================================================

/// Parameter management errors
///
/// All parameter-related operations return this error type.
/// No central error crate dependency - this is self-contained.
#[non_exhaustive]
#[derive(Error, Debug, Clone)]
pub enum ParameterError {
    /// Invalid format or content for a parameter key string
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(#[from] KeyParseError),

    /// Parameter identified by key was not found
    #[error("Parameter not found: {key}")]
    NotFound {
        /// The parameter key
        key: ParameterKey,
    },

    /// Parameter with the specified key already exists
    #[error("Parameter already exists: {key}")]
    AlreadyExists {
        /// The parameter key
        key: ParameterKey,
    },

    /// Error deserializing or processing a parameter's value
    #[error("Deserialization error for parameter '{key}': {error}")]
    DeserializationError {
        /// The parameter key
        key: ParameterKey,
        /// The error message
        error: String,
    },

    /// Error serializing a parameter's value
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Type mismatch or other type-related error when handling a parameter
    #[error("Type error for parameter '{key}': Expected {expected_type}, got {actual_details}")]
    InvalidType {
        /// The parameter key
        key: ParameterKey,
        /// Expected type
        expected_type: String,
        /// Actual type/details
        actual_details: String,
    },

    /// Validation failed for a parameter
    #[error("Validation failed for parameter '{key}': {reason}")]
    ValidationError {
        /// The parameter key
        key: ParameterKey,
        /// Reason for validation failure
        reason: String,
    },

    /// Required parameter value is missing
    #[error("Missing value for parameter '{key}'")]
    MissingValue {
        /// The parameter key
        key: ParameterKey,
    },

    /// Invalid value provided for parameter
    #[error("Invalid value for parameter '{key}': {reason}")]
    InvalidValue {
        /// The parameter key
        key: ParameterKey,
        /// Reason why value is invalid
        reason: String,
    },

    /// IO error
    #[error("IO error: {0}")]
    Io(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

// ============================================================================
// CONVENIENCE CONSTRUCTORS
// ============================================================================

impl ParameterError {
    /// Create a "not found" error
    #[must_use]
    pub fn not_found(key: ParameterKey) -> Self {
        Self::NotFound { key }
    }

    /// Create an "already exists" error
    #[must_use]
    pub fn already_exists(key: ParameterKey) -> Self {
        Self::AlreadyExists { key }
    }

    /// Create a "validation error"
    pub fn validation(key: ParameterKey, reason: impl Into<String>) -> Self {
        Self::ValidationError {
            key,
            reason: reason.into(),
        }
    }

    /// Create a "missing value" error
    #[must_use]
    pub fn missing_value(key: ParameterKey) -> Self {
        Self::MissingValue { key }
    }

    /// Create an "invalid value" error
    pub fn invalid_value(key: ParameterKey, reason: impl Into<String>) -> Self {
        Self::InvalidValue {
            key,
            reason: reason.into(),
        }
    }

    /// Create a "type error"
    pub fn type_error(
        key: ParameterKey,
        expected_type: impl Into<String>,
        actual_details: impl Into<String>,
    ) -> Self {
        Self::InvalidType {
            key,
            expected_type: expected_type.into(),
            actual_details: actual_details.into(),
        }
    }

    /// Create a "deserialization error"
    pub fn deserialization_error(key: ParameterKey, error: impl Into<String>) -> Self {
        Self::DeserializationError {
            key,
            error: error.into(),
        }
    }

    /// Create a "serialization error"
    pub fn serialization_error(error: impl Into<String>) -> Self {
        Self::SerializationError(error.into())
    }

    /// Create a "config error"
    pub fn config_error(msg: impl Into<String>) -> Self {
        Self::ConfigError(msg.into())
    }
}

// ============================================================================
// ERROR CLASSIFICATION
// ============================================================================

impl ParameterError {
    /// Get the error category for logging/metrics
    #[must_use]
    pub fn category(&self) -> &'static str {
        match self {
            Self::InvalidKeyFormat(_) => "invalid_key_format",
            Self::NotFound { .. } => "not_found",
            Self::AlreadyExists { .. } => "already_exists",
            Self::DeserializationError { .. } => "deserialization_error",
            Self::SerializationError(_) => "serialization_error",
            Self::InvalidType { .. } => "invalid_type",
            Self::ValidationError { .. } => "validation_error",
            Self::MissingValue { .. } => "missing_value",
            Self::InvalidValue { .. } => "invalid_value",
            Self::Io(_) => "io_error",
            Self::ConfigError(_) => "config_error",
        }
    }

    /// Get error code for monitoring
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidKeyFormat(_) => "PARAM_INVALID_KEY",
            Self::NotFound { .. } => "PARAM_NOT_FOUND",
            Self::AlreadyExists { .. } => "PARAM_ALREADY_EXISTS",
            Self::DeserializationError { .. } => "PARAM_DESER_ERROR",
            Self::SerializationError(_) => "PARAM_SER_ERROR",
            Self::InvalidType { .. } => "PARAM_INVALID_TYPE",
            Self::ValidationError { .. } => "PARAM_VALIDATION_ERROR",
            Self::MissingValue { .. } => "PARAM_MISSING_VALUE",
            Self::InvalidValue { .. } => "PARAM_INVALID_VALUE",
            Self::Io(_) => "PARAM_IO_ERROR",
            Self::ConfigError(_) => "PARAM_CONFIG_ERROR",
        }
    }

    /// Check if this error is retryable
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        // Only IO errors might be transient
        matches!(self, Self::Io(_) | Self::SerializationError(_))
    }

    /// Check if this is a client error (user's fault)
    #[must_use]
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidKeyFormat(_)
                | Self::NotFound { .. }
                | Self::AlreadyExists { .. }
                | Self::InvalidType { .. }
                | Self::ValidationError { .. }
                | Self::MissingValue { .. }
                | Self::InvalidValue { .. }
        )
    }

    /// Check if this is a server error (system's fault)
    #[must_use]
    pub fn is_server_error(&self) -> bool {
        matches!(
            self,
            Self::DeserializationError { .. }
                | Self::SerializationError(_)
                | Self::Io(_)
                | Self::ConfigError(_)
        )
    }
}

// ============================================================================
// EXTERNAL ERROR CONVERSIONS
// ============================================================================

/// Convert from `serde_json` errors
impl From<serde_json::Error> for ParameterError {
    fn from(error: serde_json::Error) -> Self {
        Self::SerializationError(error.to_string())
    }
}

/// Convert from `std::io::Error`
impl From<std::io::Error> for ParameterError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

// ============================================================================
// RESULT TYPE
// ============================================================================

/// Result type alias for parameter operations
pub type Result<T> = std::result::Result<T, ParameterError>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_not_found() {
        let key = ParameterKey::new("test.param").unwrap();
        let err = ParameterError::not_found(key.clone());
        assert!(matches!(err, ParameterError::NotFound { .. }));
        assert!(err.to_string().contains("test.param"));
    }

    #[test]
    fn test_error_validation() {
        let key = ParameterKey::new("test.param").unwrap();
        let err = ParameterError::validation(key, "value out of range");
        assert!(matches!(err, ParameterError::ValidationError { .. }));
        assert!(err.to_string().contains("value out of range"));
    }

    #[test]
    fn test_error_category() {
        let key = ParameterKey::new("test.param").unwrap();
        assert_eq!(
            ParameterError::not_found(key.clone()).category(),
            "not_found"
        );
        assert_eq!(
            ParameterError::validation(key.clone(), "error").category(),
            "validation_error"
        );
        assert_eq!(
            ParameterError::missing_value(key).category(),
            "missing_value"
        );
    }

    #[test]
    fn test_error_code() {
        let key = ParameterKey::new("test.param").unwrap();
        assert_eq!(
            ParameterError::not_found(key.clone()).code(),
            "PARAM_NOT_FOUND"
        );
        assert_eq!(
            ParameterError::validation(key.clone(), "error").code(),
            "PARAM_VALIDATION_ERROR"
        );
        assert_eq!(
            ParameterError::missing_value(key).code(),
            "PARAM_MISSING_VALUE"
        );
    }

    #[test]
    fn test_error_is_retryable() {
        let key = ParameterKey::new("test.param").unwrap();
        assert!(!ParameterError::not_found(key.clone()).is_retryable());
        assert!(!ParameterError::validation(key.clone(), "error").is_retryable());
        assert!(!ParameterError::missing_value(key).is_retryable());

        // IO errors are retryable
        let io_err: ParameterError = std::io::Error::from(std::io::ErrorKind::TimedOut).into();
        assert!(io_err.is_retryable());
    }

    #[test]
    fn test_error_classification() {
        let key = ParameterKey::new("test.param").unwrap();

        // Client errors
        assert!(ParameterError::not_found(key.clone()).is_client_error());
        assert!(ParameterError::validation(key.clone(), "error").is_client_error());
        assert!(!ParameterError::not_found(key.clone()).is_server_error());

        // Server errors
        let io_err: ParameterError = std::io::Error::from(std::io::ErrorKind::TimedOut).into();
        assert!(io_err.is_server_error());
        assert!(!io_err.is_client_error());
    }

    #[test]
    fn test_error_from_serde_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json");
        assert!(json_err.is_err());

        let param_err: ParameterError = json_err.unwrap_err().into();
        assert!(matches!(param_err, ParameterError::SerializationError(_)));
    }

    #[test]
    fn test_convenience_constructors() {
        let key = ParameterKey::new("test").unwrap();

        let _ = ParameterError::not_found(key.clone());
        let _ = ParameterError::already_exists(key.clone());
        let _ = ParameterError::validation(key.clone(), "bad");
        let _ = ParameterError::missing_value(key.clone());
        let _ = ParameterError::invalid_value(key.clone(), "wrong");
        let _ = ParameterError::type_error(key.clone(), "String", "i32");
        let _ = ParameterError::deserialization_error(key, "failed");
        let _ = ParameterError::serialization_error("json error");
        let _ = ParameterError::config_error("bad config");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::from(std::io::ErrorKind::NotFound);
        let param_err: ParameterError = io_err.into();

        assert!(matches!(param_err, ParameterError::Io(_)));
        assert!(param_err.is_server_error());
    }
}
