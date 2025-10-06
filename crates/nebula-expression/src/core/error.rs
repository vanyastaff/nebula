//! Error extension trait for nebula_error::NebulaError
//!
//! This module extends NebulaError with expression-specific error constructors.

use nebula_error::NebulaError;

/// Extension trait for creating expression-specific errors
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

impl ExpressionErrorExt for NebulaError {
    fn expression_syntax_error(message: impl Into<String>) -> Self {
        NebulaError::validation(format!("Expression syntax error: {}", message.into()))
    }

    fn expression_parse_error(message: impl Into<String>) -> Self {
        NebulaError::validation(format!("Expression parse error: {}", message.into()))
    }

    fn expression_eval_error(message: impl Into<String>) -> Self {
        NebulaError::internal(format!("Expression evaluation error: {}", message.into()))
    }

    fn expression_type_error(expected: impl Into<String>, found: impl Into<String>) -> Self {
        NebulaError::validation(format!(
            "Type error: expected {}, found {}",
            expected.into(),
            found.into()
        ))
    }

    fn expression_variable_not_found(name: impl Into<String>) -> Self {
        NebulaError::not_found("Variable", name.into())
    }

    fn expression_function_not_found(name: impl Into<String>) -> Self {
        NebulaError::not_found("Function", name.into())
    }

    fn expression_invalid_argument(
        function: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        NebulaError::validation(format!(
            "Invalid argument for {}: {}",
            function.into(),
            message.into()
        ))
    }

    fn expression_division_by_zero() -> Self {
        NebulaError::validation("Division by zero")
    }

    fn expression_regex_error(message: impl Into<String>) -> Self {
        NebulaError::validation(format!("Regex error: {}", message.into()))
    }

    fn expression_index_out_of_bounds(index: usize, len: usize) -> Self {
        NebulaError::validation(format!(
            "Index out of bounds: index {} is out of range for array of length {}",
            index, len
        ))
    }
}

/// Result type for expression operations
pub type ExpressionResult<T> = Result<T, NebulaError>;
