//! Standalone error types for nebula-expression
//!
//! Uses thiserror for clean, idiomatic Rust error definitions.

use thiserror::Error;

// ============================================================================
// Main Error Type
// ============================================================================

/// Expression evaluation and parsing errors
#[non_exhaustive]
#[derive(Error, Debug, nebula_error::Classify)]
pub enum ExpressionError {
    /// Syntax error in expression
    #[classify(category = "validation", code = "EXPR:SYNTAX")]
    #[error("Expression syntax error: {message}")]
    SyntaxError { message: String },

    /// Parse error
    #[classify(category = "validation", code = "EXPR:PARSE")]
    #[error("Expression parse error: {message}")]
    ParseError { message: String },

    /// Evaluation error
    #[classify(category = "internal", code = "EXPR:EVAL")]
    #[error("Expression evaluation error: {message}")]
    EvalError { message: String },

    /// Type mismatch error
    #[classify(category = "validation", code = "EXPR:TYPE")]
    #[error("Type error: expected {expected}, found {actual}")]
    TypeError { expected: String, actual: String },

    /// Variable not found
    #[classify(category = "not_found", code = "EXPR:VAR_NOT_FOUND")]
    #[error("Variable '{name}' not found")]
    VariableNotFound { name: String },

    /// Function not found
    #[classify(category = "not_found", code = "EXPR:FUNC_NOT_FOUND")]
    #[error("Function '{name}' not found")]
    FunctionNotFound { name: String },

    /// Invalid function argument
    #[classify(category = "validation", code = "EXPR:INVALID_ARG")]
    #[error("Invalid argument for {function}: {message}")]
    InvalidArgument { function: String, message: String },

    /// Division by zero
    #[classify(category = "validation", code = "EXPR:DIV_ZERO")]
    #[error("Division by zero")]
    DivisionByZero,

    /// Regex compilation or matching error
    #[classify(category = "validation", code = "EXPR:REGEX")]
    #[error("Regex error: {message}")]
    RegexError { message: String },

    /// Index out of bounds
    #[classify(category = "validation", code = "EXPR:INDEX_OOB")]
    #[error("Index out of bounds: index {index} is out of range for array of length {length}")]
    IndexOutOfBounds { index: usize, length: usize },

    /// Validation error (general)
    #[classify(category = "validation", code = "EXPR:VALIDATION")]
    #[error("Validation error: {message}")]
    Validation { message: String },

    /// Not found error (general)
    #[classify(category = "not_found", code = "EXPR:NOT_FOUND")]
    #[error("{resource_type} not found: {resource_id}")]
    NotFound {
        resource_type: String,
        resource_id: String,
    },

    /// Internal error
    #[classify(category = "internal", code = "EXPR:INTERNAL", retryable = true)]
    #[error("Internal error: {message}")]
    Internal { message: String },

    /// JSON error
    #[classify(category = "internal", code = "EXPR:JSON", retryable = true)]
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Invalid date format error
    #[classify(category = "validation", code = "EXPR:INVALID_DATE")]
    #[error("Invalid date format: {0}")]
    InvalidDate(#[from] chrono::format::ParseError),
}

impl ExpressionError {
    // ============================================================================
    // Convenience Constructors
    // ============================================================================

    /// Create a syntax error
    pub fn syntax_error(message: impl Into<String>) -> Self {
        Self::SyntaxError {
            message: message.into(),
        }
    }

    /// Create a parse error
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::ParseError {
            message: message.into(),
        }
    }

    /// Create an evaluation error
    pub fn eval_error(message: impl Into<String>) -> Self {
        Self::EvalError {
            message: message.into(),
        }
    }

    /// Create a type error
    pub fn type_error(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::TypeError {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create a variable not found error
    pub fn variable_not_found(name: impl Into<String>) -> Self {
        Self::VariableNotFound { name: name.into() }
    }

    /// Create a function not found error
    pub fn function_not_found(name: impl Into<String>) -> Self {
        Self::FunctionNotFound { name: name.into() }
    }

    /// Create an invalid argument error
    pub fn invalid_argument(function: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            function: function.into(),
            message: message.into(),
        }
    }

    /// Create a division by zero error
    pub fn division_by_zero() -> Self {
        Self::DivisionByZero
    }

    /// Create a regex error
    pub fn regex_error(message: impl Into<String>) -> Self {
        Self::RegexError {
            message: message.into(),
        }
    }

    /// Create an index out of bounds error
    pub fn index_out_of_bounds(index: usize, length: usize) -> Self {
        Self::IndexOutOfBounds { index, length }
    }

    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    /// Create a not found error
    pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        Self::NotFound {
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
        }
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

// ============================================================================
// External Error Conversions
// ============================================================================

// ============================================================================
// Result Type
// ============================================================================

/// Result type for expression operations
pub type ExpressionResult<T> = Result<T, ExpressionError>;

// ============================================================================
// Extension Trait (for convenience)
// ============================================================================

/// Extension trait for creating expression errors using method syntax
pub trait ExpressionErrorExt {
    /// Create a syntax error
    fn expression_syntax_error(message: impl Into<String>) -> Self;

    /// Create a parse error
    fn expression_parse_error(message: impl Into<String>) -> Self;

    /// Create an evaluation error
    fn expression_eval_error(message: impl Into<String>) -> Self;

    /// Create a type error
    fn expression_type_error(expected: impl Into<String>, found: impl Into<String>) -> Self;

    /// Create a variable not found error
    fn expression_variable_not_found(name: impl Into<String>) -> Self;

    /// Create a function not found error
    fn expression_function_not_found(name: impl Into<String>) -> Self;

    /// Create an invalid argument error
    fn expression_invalid_argument(function: impl Into<String>, message: impl Into<String>)
    -> Self;

    /// Create a division by zero error
    fn expression_division_by_zero() -> Self;

    /// Create a regex error
    fn expression_regex_error(message: impl Into<String>) -> Self;

    /// Create an index out of bounds error
    fn expression_index_out_of_bounds(index: usize, len: usize) -> Self;
}

impl ExpressionErrorExt for ExpressionError {
    fn expression_syntax_error(message: impl Into<String>) -> Self {
        ExpressionError::syntax_error(message)
    }

    fn expression_parse_error(message: impl Into<String>) -> Self {
        ExpressionError::parse_error(message)
    }

    fn expression_eval_error(message: impl Into<String>) -> Self {
        ExpressionError::eval_error(message)
    }

    fn expression_type_error(expected: impl Into<String>, found: impl Into<String>) -> Self {
        ExpressionError::type_error(expected, found)
    }

    fn expression_variable_not_found(name: impl Into<String>) -> Self {
        ExpressionError::variable_not_found(name)
    }

    fn expression_function_not_found(name: impl Into<String>) -> Self {
        ExpressionError::function_not_found(name)
    }

    fn expression_invalid_argument(
        function: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        ExpressionError::invalid_argument(function, message)
    }

    fn expression_division_by_zero() -> Self {
        ExpressionError::division_by_zero()
    }

    fn expression_regex_error(message: impl Into<String>) -> Self {
        ExpressionError::regex_error(message)
    }

    fn expression_index_out_of_bounds(index: usize, len: usize) -> Self {
        ExpressionError::index_out_of_bounds(index, len)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_error::Classify;

    #[test]
    fn test_error_creation() {
        let error = ExpressionError::syntax_error("unexpected token");
        assert!(error.to_string().contains("syntax error"));
    }

    #[test]
    fn test_type_error() {
        let error = ExpressionError::type_error("number", "string");
        assert!(error.to_string().contains("expected number"));
        assert!(error.to_string().contains("found string"));
    }

    #[test]
    fn test_error_codes() {
        assert_eq!(ExpressionError::syntax_error("test").code(), "EXPR:SYNTAX");
        assert_eq!(ExpressionError::division_by_zero().code(), "EXPR:DIV_ZERO");
    }

    #[test]
    fn test_retryable() {
        assert!(!ExpressionError::syntax_error("test").is_retryable());
        assert!(ExpressionError::internal("test").is_retryable());
    }
}
