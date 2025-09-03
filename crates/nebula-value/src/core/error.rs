use thiserror::Error;

/// Type alias for Result with ValueError
pub type ValueResult<T> = Result<T, ValueError>;

/// Main error type for Value operations
#[derive(Debug, Error, Clone)]
pub enum ValueError {
    /// Type-related errors
    #[error(transparent)]
    Type(#[from] TypeError),

    /// Conversion errors
    #[error(transparent)]
    Conversion(#[from] ConversionError),

    /// Access errors (index, key, path)
    #[error(transparent)]
    Access(#[from] AccessError),

    /// Validation errors
    #[error(transparent)]
    Validation(#[from] ValidationError),

    /// Parse errors
    #[error(transparent)]
    Parse(#[from] ParseError),

    /// Operation errors
    #[error(transparent)]
    Operation(#[from] OperationError),

    /// IO errors
    #[error("IO error: {0}")]
    Io(String),

    /// Custom error with message
    #[error("{0}")]
    Custom(String),
}

/// Type-related errors
#[derive(Debug, Error, Clone)]
pub enum TypeError {
    /// Type mismatch
    #[error("Type mismatch: expected {expected}, got {actual}")]
    Mismatch { expected: String, actual: String },

    /// Incompatible types for operation
    #[error("Incompatible types: {left} and {right}")]
    Incompatible { left: String, right: String },

    /// Invalid type for operation
    #[error("Invalid type '{ty}' for operation '{operation}'")]
    InvalidForOperation { ty: String, operation: String },

    /// Unknown type
    #[error("Unknown type: {0}")]
    Unknown(String),
}

/// Conversion errors
#[derive(Debug, Error, Clone)]
pub enum ConversionError {
    /// Failed to convert between types
    #[error("Cannot convert from {from} to {to}")]
    CannotConvert { from: String, to: String },

    /// Failed to convert with value details
    #[error("Cannot convert {from} '{value}' to {to}")]
    CannotConvertValue { from: String, to: String, value: String },

    /// Loss of precision
    #[error("Conversion would lose precision: {details}")]
    PrecisionLoss { details: String },

    /// Overflow during conversion
    #[error("Overflow converting {value} to {target_type}")]
    Overflow { value: String, target_type: String },
}

/// Access errors (index, key, path)
#[derive(Debug, Error, Clone)]
pub enum AccessError {
    /// Array index out of bounds
    #[error("Index {index} out of bounds (length: {length})")]
    IndexOutOfBounds { index: usize, length: usize },

    /// Key not found in object
    #[error("Key '{key}' not found")]
    KeyNotFound { key: String },

    /// Invalid path
    #[error("Invalid path: {path}")]
    InvalidPath { path: String },

    /// Path not found
    #[error("Path '{path}' not found")]
    PathNotFound { path: String },

    /// Cannot access field on non-object
    #[error("Cannot access field '{field}' on {value_type}")]
    FieldAccessOnNonObject { field: String, value_type: String },

    /// Cannot index non-array
    #[error("Cannot index {value_type} (not an array)")]
    IndexOnNonArray { value_type: String },
}

/// Validation errors
#[derive(Debug, Error, Clone)]
pub enum ValidationError {
    /// Required value is missing
    #[error("Required value is missing: {field}")]
    Required { field: String },

    /// Value is out of range
    #[error("Value {value} is out of range [{min}, {max}]")]
    OutOfRange { value: String, min: String, max: String },

    /// Invalid length
    #[error("Invalid length {actual}, expected {constraint}")]
    InvalidLength { actual: usize, constraint: String },

    /// Pattern mismatch
    #[error("Value '{value}' doesn't match pattern '{pattern}'")]
    PatternMismatch { value: String, pattern: String },

    /// Custom validation failure
    #[error("Validation failed: {reason}")]
    Failed { reason: String },
}

/// Parse errors
#[derive(Debug, Error, Clone)]
pub enum ParseError {
    /// Invalid integer format
    #[error("Invalid integer: {input}")]
    InvalidInteger { input: String },

    /// Invalid float format
    #[error("Invalid float: {input}")]
    InvalidFloat { input: String },

    /// Invalid boolean format
    #[error("Invalid boolean: {input}")]
    InvalidBoolean { input: String },

    /// Invalid date/time format
    #[error("Invalid {format_type}: {input}")]
    InvalidDateTime { format_type: String, input: String },

    /// Invalid JSON
    #[error("Invalid JSON: {details}")]
    InvalidJson { details: String },

    /// Invalid format
    #[error("Invalid {format_type} format: {input}")]
    InvalidFormat { format_type: String, input: String },

    /// Unexpected end of input
    #[error("Unexpected end of input")]
    UnexpectedEnd,

    /// Unexpected character
    #[error("Unexpected character '{ch}' at position {pos}")]
    UnexpectedChar { ch: char, pos: usize },
}

/// Operation errors
#[derive(Debug, Error, Clone)]
pub enum OperationError {
    /// Division by zero
    #[error("Division by zero")]
    DivisionByZero,

    /// Operation not supported
    #[error("Operation '{operation}' not supported for {value_type}")]
    NotSupported { operation: String, value_type: String },

    /// Invalid operands
    #[error("Invalid operands for {operation}: {details}")]
    InvalidOperands { operation: String, details: String },

    /// Overflow in arithmetic
    #[error("Arithmetic overflow in {operation}")]
    ArithmeticOverflow { operation: String },

    /// Not a finite number
    #[error("Result is not a finite number")]
    NotFinite,
}

// ==================== Convenience constructors ====================

impl ValueError {
    /// Create a custom error
    pub fn custom<S: Into<String>>(msg: S) -> Self {
        Self::Custom(msg.into())
    }

    /// Create an IO error
    pub fn io<S: Into<String>>(msg: S) -> Self {
        Self::Io(msg.into())
    }
}

impl TypeError {
    /// Create a type mismatch error
    pub fn mismatch<S1, S2>(expected: S1, actual: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Mismatch { expected: expected.into(), actual: actual.into() }
    }

    /// Create an incompatible types error
    pub fn incompatible<S1, S2>(left: S1, right: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Incompatible { left: left.into(), right: right.into() }
    }

    /// Create an invalid type for operation error
    pub fn invalid_for_operation<S1, S2>(ty: S1, operation: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::InvalidForOperation { ty: ty.into(), operation: operation.into() }
    }
}

impl ConversionError {
    /// Create a cannot convert error
    pub fn cannot_convert<S1, S2>(from: S1, to: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::CannotConvert { from: from.into(), to: to.into() }
    }

    /// Create a cannot convert with value error
    pub fn cannot_convert_value<S1, S2, S3>(from: S1, to: S2, value: S3) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        S3: Into<String>,
    {
        Self::CannotConvertValue { from: from.into(), to: to.into(), value: value.into() }
    }

    /// Create an overflow error
    pub fn overflow<S1, S2>(value: S1, target_type: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Overflow { value: value.into(), target_type: target_type.into() }
    }
}

impl AccessError {
    /// Create an index out of bounds error
    pub fn index_out_of_bounds(index: usize, length: usize) -> Self {
        Self::IndexOutOfBounds { index, length }
    }

    /// Create a key not found error
    pub fn key_not_found<S: Into<String>>(key: S) -> Self {
        Self::KeyNotFound { key: key.into() }
    }

    /// Create a path not found error
    pub fn path_not_found<S: Into<String>>(path: S) -> Self {
        Self::PathNotFound { path: path.into() }
    }

    /// Create an invalid path error
    pub fn invalid_path<S: Into<String>>(path: S) -> Self {
        Self::InvalidPath { path: path.into() }
    }
}

impl ValidationError {
    /// Create a required field error
    pub fn required<S: Into<String>>(field: S) -> Self {
        Self::Required { field: field.into() }
    }

    /// Create an out of range error
    pub fn out_of_range<S1, S2, S3>(value: S1, min: S2, max: S3) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        S3: Into<String>,
    {
        Self::OutOfRange { value: value.into(), min: min.into(), max: max.into() }
    }

    /// Create a pattern mismatch error
    pub fn pattern_mismatch<S1, S2>(value: S1, pattern: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::PatternMismatch { value: value.into(), pattern: pattern.into() }
    }

    /// Create a validation failed error
    pub fn failed<S: Into<String>>(reason: S) -> Self {
        Self::Failed { reason: reason.into() }
    }
}

impl ParseError {
    /// Create an invalid integer error
    pub fn invalid_integer<S: Into<String>>(input: S) -> Self {
        Self::InvalidInteger { input: input.into() }
    }

    /// Create an invalid float error
    pub fn invalid_float<S: Into<String>>(input: S) -> Self {
        Self::InvalidFloat { input: input.into() }
    }

    /// Create an invalid boolean error
    pub fn invalid_boolean<S: Into<String>>(input: S) -> Self {
        Self::InvalidBoolean { input: input.into() }
    }

    /// Create an invalid format error
    pub fn invalid_format<S1, S2>(format_type: S1, input: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::InvalidFormat { format_type: format_type.into(), input: input.into() }
    }
}

impl OperationError {
    /// Create a not supported error
    pub fn not_supported<S1, S2>(operation: S1, value_type: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::NotSupported { operation: operation.into(), value_type: value_type.into() }
    }

    /// Create an invalid operands error
    pub fn invalid_operands<S1, S2>(operation: S1, details: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::InvalidOperands { operation: operation.into(), details: details.into() }
    }

    /// Create an arithmetic overflow error
    pub fn arithmetic_overflow<S: Into<String>>(operation: S) -> Self {
        Self::ArithmeticOverflow { operation: operation.into() }
    }
}

// ==================== Backwards compatibility helpers ====================

impl ValueError {
    // These are shortcuts for common error patterns to maintain backwards compatibility

    /// Create a type mismatch error (shortcut)
    pub fn type_mismatch<S1, S2>(expected: S1, actual: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Type(TypeError::mismatch(expected, actual))
    }

    /// Create an incompatible types error (shortcut)
    pub fn incompatible_types<S1, S2>(left: S1, right: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Type(TypeError::incompatible(left, right))
    }

    /// Create a conversion error (shortcut)
    pub fn cannot_convert<S1, S2>(from: S1, to: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Conversion(ConversionError::cannot_convert(from, to))
    }

    /// Create an index out of bounds error (shortcut)
    pub fn index_out_of_bounds(index: usize, length: usize) -> Self {
        Self::Access(AccessError::index_out_of_bounds(index, length))
    }

    /// Create a key not found error (shortcut)
    pub fn key_not_found<S: Into<String>>(key: S) -> Self {
        Self::Access(AccessError::key_not_found(key))
    }

    /// Create a division by zero error (shortcut)
    pub fn division_by_zero() -> Self {
        Self::Operation(OperationError::DivisionByZero)
    }

    /// Create an unsupported operation error (shortcut)
    pub fn unsupported_operation<S1, S2>(operation: S1, value_type: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Operation(OperationError::not_supported(operation, value_type))
    }

    /// Create an invalid format error (shortcut)
    pub fn invalid_format<S1, S2>(format_type: S1, input: S2) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self::Parse(ParseError::invalid_format(format_type, input))
    }

    /// Create a validation failed error (shortcut)
    pub fn validation_failed<S: Into<String>>(reason: S) -> Self {
        Self::Validation(ValidationError::failed(reason))
    }
}

// ==================== Error display helpers ====================

/// Helper trait for creating detailed error messages
pub trait ErrorContext {
    /// Add context to the error
    fn context<S: Into<String>>(self, ctx: S) -> ValueError;

    /// Add field context
    fn for_field<S: Into<String>>(self, field: S) -> ValueError;
}

impl ErrorContext for ValueError {
    fn context<S: Into<String>>(self, ctx: S) -> ValueError {
        ValueError::custom(format!("{}: {}", ctx.into(), self))
    }

    fn for_field<S: Into<String>>(self, field: S) -> ValueError {
        ValueError::custom(format!("Field '{}': {}", field.into(), self))
    }
}

// ==================== Result helpers ====================

/// Extension trait for Result types
pub trait ResultExt<T> {
    /// Convert to ValueError with custom message
    fn or_error<S: Into<String>>(self, msg: S) -> ValueResult<T>;

    /// Add context to error
    fn with_context<S: Into<String>, F>(self, f: F) -> ValueResult<T>
    where
        F: FnOnce() -> S;
}

impl<T, E> ResultExt<T> for Result<T, E>
where
    E: std::error::Error,
{
    fn or_error<S: Into<String>>(self, msg: S) -> ValueResult<T> {
        self.map_err(|_| ValueError::custom(msg))
    }

    fn with_context<S: Into<String>, F>(self, f: F) -> ValueResult<T>
    where
        F: FnOnce() -> S,
    {
        self.map_err(|e| ValueError::custom(format!("{}: {}", f().into(), e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = ValueError::type_mismatch("string", "integer");
        assert!(matches!(err, ValueError::Type(TypeError::Mismatch { .. })));

        let err = ValueError::index_out_of_bounds(5, 3);
        assert!(matches!(err, ValueError::Access(AccessError::IndexOutOfBounds { .. })));

        let err = ValueError::division_by_zero();
        assert!(matches!(err, ValueError::Operation(OperationError::DivisionByZero)));
    }

    #[test]
    fn test_error_display() {
        let err = TypeError::mismatch("string", "integer");
        assert_eq!(err.to_string(), "Type mismatch: expected string, got integer");

        let err = AccessError::key_not_found("name");
        assert_eq!(err.to_string(), "Key 'name' not found");

        let err = ValidationError::out_of_range("10", "0", "5");
        assert_eq!(err.to_string(), "Value 10 is out of range [0, 5]");
    }

    #[test]
    fn test_error_context() {
        let err = ValueError::type_mismatch("string", "integer");
        let err_with_context = err.context("Processing user data");
        assert!(err_with_context.to_string().contains("Processing user data"));
    }
}
