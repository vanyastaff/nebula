//! Value Error Types (Standalone)
//!
//! Following the pattern used by Tokio, Bevy, and other major Rust projects.

use thiserror::Error;

// ============================================================================
// MAIN ERROR TYPE
// ============================================================================

/// Value management errors
///
/// All value-related operations return this error type.
/// No central error crate dependency - this is self-contained.
#[non_exhaustive]
#[derive(Error, Debug, Clone)]
pub enum ValueError {
    /// Type mismatch between expected and actual types
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    /// Value limit exceeded
    #[error("{limit} exceeded: {actual} > {max}")]
    LimitExceeded {
        limit: String,
        max: usize,
        actual: usize,
    },

    /// Conversion between types failed
    #[error("Cannot convert from {from} to {to}")]
    ConversionError { from: String, to: String },

    /// Array index out of bounds
    #[error("Index {index} out of bounds (length: {length})")]
    IndexOutOfBounds { index: usize, length: usize },

    /// Object key not found
    #[error("Key not found: '{key}'")]
    KeyNotFound { key: String },

    /// Path not found in nested value
    #[error("Path not found: {path}")]
    PathNotFound { path: String },

    /// Parse error for specific format
    #[error("Invalid {format_type} format: {input}")]
    ParseError {
        format_type: String,
        input: String,
        position: Option<usize>,
    },

    /// Operation not supported for this value type
    #[error("Operation '{operation}' not supported for {value_type}")]
    OperationNotSupported {
        operation: String,
        value_type: String,
    },

    /// Numeric overflow
    #[error("Numeric overflow in {operation}: value {value}")]
    Overflow { operation: String, value: String },

    /// Feature not enabled
    #[error("Feature '{feature}' not enabled for operation: {operation}")]
    FeatureNotEnabled { feature: String, operation: String },

    /// Value out of range
    #[error("Value {value} out of range [{min}, {max}]")]
    OutOfRange {
        value: String,
        min: String,
        max: String,
    },

    /// Validation error
    #[error("Validation failed: {reason}")]
    ValidationFailed { reason: String },

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// Context information (nested error with additional info)
    #[error("{message}: {source}")]
    WithContext {
        message: String,
        #[source]
        source: Box<ValueError>,
    },
}

// ============================================================================
// CONVENIENCE CONSTRUCTORS
// ============================================================================

impl ValueError {
    /// Create a type mismatch error
    pub fn type_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::TypeMismatch {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create a limit exceeded error
    pub fn limit_exceeded(limit: impl Into<String>, max: usize, actual: usize) -> Self {
        Self::LimitExceeded {
            limit: limit.into(),
            max,
            actual,
        }
    }

    /// Create a conversion error
    pub fn conversion_error(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::ConversionError {
            from: from.into(),
            to: to.into(),
        }
    }

    /// Create an index out of bounds error
    pub fn index_out_of_bounds(index: usize, length: usize) -> Self {
        Self::IndexOutOfBounds { index, length }
    }

    /// Create a key not found error
    pub fn key_not_found(key: impl Into<String>) -> Self {
        Self::KeyNotFound { key: key.into() }
    }

    /// Create a path not found error
    pub fn path_not_found(path: impl Into<String>) -> Self {
        Self::PathNotFound { path: path.into() }
    }

    /// Create a parse error
    pub fn parse_error(format_type: impl Into<String>, input: impl Into<String>) -> Self {
        Self::ParseError {
            format_type: format_type.into(),
            input: input.into(),
            position: None,
        }
    }

    /// Create a parse error with position
    pub fn parse_error_at(
        format_type: impl Into<String>,
        input: impl Into<String>,
        position: usize,
    ) -> Self {
        Self::ParseError {
            format_type: format_type.into(),
            input: input.into(),
            position: Some(position),
        }
    }

    /// Create an operation not supported error
    pub fn operation_not_supported(
        operation: impl Into<String>,
        value_type: impl Into<String>,
    ) -> Self {
        Self::OperationNotSupported {
            operation: operation.into(),
            value_type: value_type.into(),
        }
    }

    /// Create a numeric overflow error
    pub fn overflow(operation: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Overflow {
            operation: operation.into(),
            value: value.into(),
        }
    }

    /// Create a feature not enabled error
    pub fn feature_not_enabled(feature: impl Into<String>, operation: impl Into<String>) -> Self {
        Self::FeatureNotEnabled {
            feature: feature.into(),
            operation: operation.into(),
        }
    }

    /// Create an out of range error
    pub fn out_of_range(
        value: impl Into<String>,
        min: impl Into<String>,
        max: impl Into<String>,
    ) -> Self {
        Self::OutOfRange {
            value: value.into(),
            min: min.into(),
            max: max.into(),
        }
    }

    /// Create a validation error
    pub fn validation(reason: impl Into<String>) -> Self {
        Self::ValidationFailed {
            reason: reason.into(),
        }
    }

    /// Add context to an error
    pub fn with_context(self, message: impl Into<String>) -> Self {
        Self::WithContext {
            message: message.into(),
            source: Box::new(self),
        }
    }

    /// Add path context
    pub fn at_path(self, path: impl Into<String>) -> Self {
        self.with_context(format!("at path: {}", path.into()))
    }

    /// Add key context
    pub fn at_key(self, key: impl Into<String>) -> Self {
        self.with_context(format!("at key: '{}'", key.into()))
    }

    /// Add index context
    pub fn at_index(self, index: usize) -> Self {
        self.with_context(format!("at index: {}", index))
    }
}

// ============================================================================
// ERROR CLASSIFICATION
// ============================================================================

impl ValueError {
    /// Get error code for monitoring
    pub fn code(&self) -> &'static str {
        match self {
            Self::TypeMismatch { .. } => "VALUE_TYPE_MISMATCH",
            Self::LimitExceeded { .. } => "VALUE_LIMIT_EXCEEDED",
            Self::ConversionError { .. } => "VALUE_CONVERSION_ERROR",
            Self::IndexOutOfBounds { .. } => "VALUE_INDEX_OUT_OF_BOUNDS",
            Self::KeyNotFound { .. } => "VALUE_KEY_NOT_FOUND",
            Self::PathNotFound { .. } => "VALUE_PATH_NOT_FOUND",
            Self::ParseError { .. } => "VALUE_PARSE_ERROR",
            Self::OperationNotSupported { .. } => "VALUE_OPERATION_NOT_SUPPORTED",
            Self::Overflow { .. } => "VALUE_OVERFLOW",
            Self::FeatureNotEnabled { .. } => "VALUE_FEATURE_NOT_ENABLED",
            Self::OutOfRange { .. } => "VALUE_OUT_OF_RANGE",
            Self::ValidationFailed { .. } => "VALUE_VALIDATION_FAILED",
            Self::SerializationError(_) => "VALUE_SERIALIZATION_ERROR",
            Self::DeserializationError(_) => "VALUE_DESERIALIZATION_ERROR",
            Self::WithContext { source, .. } => source.code(),
        }
    }

    /// Check if this is a client error (user's fault)
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            Self::TypeMismatch { .. }
                | Self::ConversionError { .. }
                | Self::IndexOutOfBounds { .. }
                | Self::KeyNotFound { .. }
                | Self::PathNotFound { .. }
                | Self::ParseError { .. }
                | Self::OperationNotSupported { .. }
                | Self::Overflow { .. }
                | Self::OutOfRange { .. }
                | Self::ValidationFailed { .. }
                | Self::LimitExceeded { .. }
        )
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        // Value errors are generally not retryable (client errors)
        matches!(
            self,
            Self::SerializationError(_) | Self::DeserializationError(_)
        )
    }
}

// ============================================================================
// EXTERNAL ERROR CONVERSIONS
// ============================================================================

/// Convert from serde_json errors
impl From<serde_json::Error> for ValueError {
    fn from(error: serde_json::Error) -> Self {
        Self::SerializationError(error.to_string())
    }
}

// ============================================================================
// RESULT TYPE
// ============================================================================

/// Result type alias for value operations
pub type Result<T> = std::result::Result<T, ValueError>;

/// Backward compatibility alias
pub type ValueResult<T> = Result<T>;

// ============================================================================
// RESULT EXTENSION TRAIT
// ============================================================================

/// Extension trait for Result types (value-specific)
pub trait ValueResultExt<T> {
    /// Convert to ValueError with custom message
    fn or_value_error<S: Into<String>>(self, msg: S) -> Result<T>;

    /// Add context to error
    fn with_value_context<S: Into<String>, F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> S;
}

impl<T, E> ValueResultExt<T> for std::result::Result<T, E>
where
    E: std::error::Error,
{
    fn or_value_error<S: Into<String>>(self, msg: S) -> Result<T> {
        self.map_err(|_| ValueError::validation(msg))
    }

    fn with_value_context<S: Into<String>, F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> S,
    {
        self.map_err(|e| ValueError::validation(format!("{}: {}", f().into(), e)))
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_mismatch() {
        let err = ValueError::type_mismatch("string", "integer");
        assert_eq!(err.code(), "VALUE_TYPE_MISMATCH");
        assert!(err.is_client_error());
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_index_out_of_bounds() {
        let err = ValueError::index_out_of_bounds(5, 3);
        assert_eq!(err.code(), "VALUE_INDEX_OUT_OF_BOUNDS");
        assert!(err.to_string().contains("5"));
        assert!(err.to_string().contains("3"));
    }

    #[test]
    fn test_with_context() {
        let err = ValueError::key_not_found("test")
            .at_path("root.data")
            .at_index(0);

        let msg = err.to_string();
        assert!(msg.contains("test"));
        assert!(msg.contains("root.data"));
        assert!(msg.contains("index: 0"));
    }

    #[test]
    fn test_limit_exceeded() {
        let err = ValueError::limit_exceeded("max_array_length", 1000, 1500);
        assert!(err.to_string().contains("1000"));
        assert!(err.to_string().contains("1500"));
    }

    #[test]
    fn test_parse_error() {
        let err = ValueError::parse_error("JSON", "invalid json");
        assert_eq!(err.code(), "VALUE_PARSE_ERROR");

        let err = ValueError::parse_error_at("JSON", "invalid", 10);
        assert!(matches!(
            err,
            ValueError::ParseError {
                position: Some(10),
                ..
            }
        ));
    }

    #[test]
    fn test_result_ext() {
        let result: std::result::Result<(), std::io::Error> =
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let value_result = result.or_value_error("Custom error");

        assert!(value_result.is_err());
        let err = value_result.unwrap_err();
        assert!(err.to_string().contains("Custom error"));
    }

    #[test]
    fn test_from_serde_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json");
        assert!(json_err.is_err());

        let value_err: ValueError = json_err.unwrap_err().into();
        assert!(matches!(value_err, ValueError::SerializationError(_)));
    }

    #[test]
    fn test_conversion_error() {
        let err = ValueError::conversion_error("String", "i32");
        assert!(err.is_client_error());
        assert_eq!(err.code(), "VALUE_CONVERSION_ERROR");
    }
}
