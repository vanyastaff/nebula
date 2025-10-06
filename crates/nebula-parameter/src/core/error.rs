use nebula_core::{KeyParseError, ParameterKey};
use nebula_error::prelude::*;

/// Main error type for parameter operations
#[derive(ThisError, Debug, Clone)]
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
}

impl ParameterError {
    /// Create a "not found" error
    pub fn not_found(key: ParameterKey) -> Self {
        Self::NotFound { key }
    }

    /// Create an "already exists" error
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

    /// Get the error category for logging/metrics
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
        }
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        // Parameter errors are generally not retryable as they're client errors
        false
    }
}

/// Result type alias for parameter operations
pub type Result<T> = std::result::Result<T, ParameterError>;

/// Convert from `serde_json` errors
impl From<serde_json::Error> for ParameterError {
    fn from(error: serde_json::Error) -> Self {
        Self::SerializationError(error.to_string())
    }
}

/// Convert ParameterError to NebulaError for unified error handling
impl From<ParameterError> for NebulaError {
    fn from(err: ParameterError) -> Self {
        match err {
            // Client errors (4xx equivalent) - not retryable
            ParameterError::NotFound { key } => {
                NebulaError::not_found("parameter", key.to_string())
            }
            ParameterError::AlreadyExists { key } => {
                NebulaError::validation(format!("Parameter already exists: {}", key))
            }
            ParameterError::InvalidKeyFormat(err) => {
                NebulaError::validation(format!("Invalid key format: {}", err))
            }
            ParameterError::ValidationError { key, reason } => NebulaError::validation(format!(
                "Validation failed for parameter '{}': {}",
                key, reason
            )),
            ParameterError::MissingValue { key } => {
                NebulaError::validation(format!("Missing value for parameter '{}'", key))
            }
            ParameterError::InvalidValue { key, reason } => NebulaError::validation(format!(
                "Invalid value for parameter '{}': {}",
                key, reason
            )),
            ParameterError::InvalidType {
                key,
                expected_type,
                actual_details,
            } => NebulaError::validation(format!(
                "Type error for parameter '{}': Expected {}, got {}",
                key, expected_type, actual_details
            )),
            ParameterError::DeserializationError { key, error } => NebulaError::validation(
                format!("Deserialization error for parameter '{}': {}", key, error),
            ),
            ParameterError::SerializationError(msg) => {
                NebulaError::internal(format!("Serialization error: {}", msg))
            }
        }
    }
}

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
    fn test_error_is_retryable() {
        let key = ParameterKey::new("test.param").unwrap();
        assert!(!ParameterError::not_found(key.clone()).is_retryable());
        assert!(!ParameterError::validation(key.clone(), "error").is_retryable());
        assert!(!ParameterError::missing_value(key).is_retryable());
    }

    #[test]
    fn test_error_from_serde_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json");
        assert!(json_err.is_err());

        let param_err: ParameterError = json_err.unwrap_err().into();
        assert!(matches!(param_err, ParameterError::SerializationError(_)));
    }

    #[test]
    fn test_error_to_nebula_error() {
        let key = ParameterKey::new("test.param").unwrap();
        let param_err = ParameterError::not_found(key);
        let nebula_err: NebulaError = param_err.into();

        assert!(nebula_err.is_client_error());
        assert_eq!(nebula_err.error_code(), "NOT_FOUND_ERROR");
    }
}
