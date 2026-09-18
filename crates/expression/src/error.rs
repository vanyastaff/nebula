//! Standalone error types for nebula-expression
//!
//! Uses thiserror for clean, idiomatic Rust error definitions.
//!
//! # Taxonomy
//!
//! Every variant is constructed somewhere in the crate and carries its meaning
//! in fields, not in a formatted string. The split is: what the author wrote
//! wrong (`SyntaxError`, `ParseError`, `TypeError`, `InvalidArgument`,
//! `InvalidDate`, `InvalidJson`, `EvalError`), what was not found
//! (`VariableNotFound`, `FunctionNotFound`, `PropertyNotFound`, `KeyNotFound`),
//! what policy forbade (`FunctionNotAllowed`), what a finite budget stopped
//! (`StepLimitExceeded`, `DepthExceeded`, `ResourceLimitExceeded`,
//! `BuiltinOutputLimitExceeded`), arithmetic outcomes (`DivisionByZero`,
//! `NumericOverflow`, `NonFiniteNumber`), and what the crate itself broke
//! (`Internal`).
//!
//! # Redaction
//!
//! Property and variable names come from the template source and may be
//! echoed. Runtime lookup keys come from data — they may carry credential
//! material — so [`ExpressionError::KeyNotFound`] deliberately has no payload:
//! it cannot leak by construction.

use thiserror::Error;

use crate::builtins::BuiltinOutputLimit;

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
    #[error("Expression parse error{}: {message}", parse_error_position(position))]
    ParseError {
        /// Position in the template source, when the failing construct has one.
        ///
        /// Structured so callers can render source context themselves
        /// (`error_formatter::format_template_error`) instead of parsing a
        /// pre-rendered diagnostic. `None` for raw-grammar parse failures.
        position: Option<crate::template::Position>,
        message: String,
    },

    /// Evaluation error: the expression is well-formed but wrong at runtime,
    /// and the failure is not one of the more specific variants below.
    ///
    /// The author can fix these by changing the expression or its input;
    /// they are never retryable and never a crate defect. Crate defects go to
    /// [`ExpressionError::Internal`].
    #[classify(category = "validation", code = "EXPR:EVAL")]
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

    /// A property named in the template source is absent.
    ///
    /// `property` comes from the expression text, so echoing it is safe.
    /// A key computed from data goes to [`ExpressionError::KeyNotFound`].
    #[classify(category = "not_found", code = "EXPR:PROPERTY_NOT_FOUND")]
    #[error("Property '{property}' not found")]
    PropertyNotFound { property: String },

    /// An object key computed from input data is absent.
    ///
    /// Deliberately carries no key: the value was derived from data, which may
    /// hold credential material, and diagnostics must never echo it.
    #[classify(category = "not_found", code = "EXPR:KEY_NOT_FOUND")]
    #[error("Object key not found")]
    KeyNotFound,

    /// The evaluation policy denies this function.
    ///
    /// Distinct from [`ExpressionError::FunctionNotFound`]: the function
    /// exists and the registry can call it, but the effective policy forbids
    /// it for this evaluation.
    #[classify(category = "authorization", code = "EXPR:FUNC_DENIED")]
    #[error("Function '{name}' is denied by policy")]
    FunctionNotAllowed { name: String },

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

    /// A date, timestamp, or date string could not be interpreted.
    #[classify(category = "validation", code = "EXPR:INVALID_DATE")]
    #[error("Invalid date: {message}")]
    InvalidDate { message: String },

    /// A JSON document supplied at runtime could not be parsed.
    ///
    /// Distinct from [`ExpressionError::ParseError`], which is the expression
    /// grammar: this is data, parsed by `parse_json`.
    #[classify(category = "validation", code = "EXPR:INVALID_JSON")]
    #[error("Invalid JSON: {message}")]
    InvalidJson { message: String },

    /// Internal error: a crate invariant was violated.
    ///
    /// Not retryable: the same expression cannot succeed on a retry, only on a
    /// fixed build or a different input.
    #[classify(category = "internal", code = "EXPR:INTERNAL")]
    #[error("Internal error: {message}")]
    Internal { message: String },

    /// Step budget exhausted: per-call evaluation cap (`max_eval_steps`)
    /// has been hit. Carries `limit` and `actual` so callers can
    /// distinguish a tight policy from a runaway expression and reason
    /// about whether to relax the limit or shrink the input.
    #[classify(category = "validation", code = "EXPR:STEP_LIMIT")]
    #[error("Step budget exhausted: actual={actual} > limit={limit}")]
    StepLimitExceeded { limit: usize, actual: usize },

    /// Recursion depth exhausted: the per-call AST depth tracker
    /// (`MAX_RECURSION_DEPTH`) has been hit. Distinguishes a hostile
    /// stack-blowing input from a legitimate `EvalError`.
    #[classify(category = "validation", code = "EXPR:DEPTH_LIMIT")]
    #[error("Recursion depth exhausted: actual={actual} >= limit={limit}")]
    DepthExceeded { limit: usize, actual: usize },

    /// A registered builtin result exceeded its mandatory output policy.
    #[classify(category = "validation", code = "EXPR:BUILTIN_OUTPUT_LIMIT")]
    #[error("Builtin output {dimension} limit exceeded: actual={actual} > limit={limit}")]
    BuiltinOutputLimitExceeded {
        /// Output dimension that exceeded its limit.
        dimension: BuiltinOutputLimit,
        /// Configured finite ceiling.
        limit: usize,
        /// Attempted output size.
        actual: usize,
    },

    /// An embedded expression failed at a position in a compiled template.
    #[classify(category = "validation", code = "EXPR:TEMPLATE_EVAL")]
    #[error("Template evaluation failed at {position}: {source}")]
    TemplateEvaluation {
        position: crate::template::Position,
        #[source]
        source: Box<ExpressionError>,
    },

    /// Compilation or output exceeded a hard resource bound.
    #[classify(category = "validation", code = "EXPR:RESOURCE_LIMIT")]
    #[error("Expression {resource} limit exceeded: actual={actual} > limit={limit}")]
    ResourceLimitExceeded {
        resource: &'static str,
        limit: usize,
        actual: usize,
    },

    /// Integer arithmetic exceeded the representable JSON integer range.
    #[classify(category = "validation", code = "EXPR:NUMERIC_OVERFLOW")]
    #[error("Integer overflow in {operation}")]
    NumericOverflow { operation: &'static str },

    /// A numeric conversion or operation produced infinity or NaN.
    #[classify(category = "validation", code = "EXPR:NONFINITE")]
    #[error("Non-finite number in {operation}")]
    NonFiniteNumber { operation: &'static str },
}

/// Display helper for the optional position on [`ExpressionError::ParseError`].
fn parse_error_position(position: &Option<crate::template::Position>) -> String {
    match position {
        Some(position) => format!(" at {position}"),
        None => String::new(),
    }
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

    /// Create a parse error without a known template position.
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::ParseError {
            position: None,
            message: message.into(),
        }
    }

    /// Create a parse error carrying its position in the template source.
    pub fn parse_error_at(position: crate::template::Position, message: impl Into<String>) -> Self {
        Self::ParseError {
            position: Some(position),
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

    /// Create a property-not-found error for an authored property name.
    pub fn property_not_found(property: impl Into<String>) -> Self {
        Self::PropertyNotFound {
            property: property.into(),
        }
    }

    /// Create a key-not-found error for a key computed from input data.
    pub fn key_not_found() -> Self {
        Self::KeyNotFound
    }

    /// Create a policy-denial error.
    pub fn function_not_allowed(name: impl Into<String>) -> Self {
        Self::FunctionNotAllowed { name: name.into() }
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

    /// Create an invalid-date error.
    pub fn invalid_date(message: impl Into<String>) -> Self {
        Self::InvalidDate {
            message: message.into(),
        }
    }

    /// Create an invalid-JSON error.
    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self::InvalidJson {
            message: message.into(),
        }
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Create a step-limit-exceeded error.
    pub fn step_limit_exceeded(limit: usize, actual: usize) -> Self {
        Self::StepLimitExceeded { limit, actual }
    }

    /// Create a recursion-depth-exceeded error.
    pub fn depth_exceeded(limit: usize, actual: usize) -> Self {
        Self::DepthExceeded { limit, actual }
    }

    /// Create a builtin-output-limit error.
    pub fn builtin_output_limit_exceeded(
        dimension: BuiltinOutputLimit,
        limit: usize,
        actual: usize,
    ) -> Self {
        Self::BuiltinOutputLimitExceeded {
            dimension,
            limit,
            actual,
        }
    }
}

// ============================================================================
// Result Type
// ============================================================================

/// Result type for expression operations
pub type ExpressionResult<T> = Result<T, ExpressionError>;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use nebula_error::Classify;

    use super::*;

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
    fn internal_is_not_retryable() {
        // An invariant breach is not fixed by retrying the same input; it is
        // fixed by a different build. Retrying it would only burn budget.
        assert!(!ExpressionError::internal("test").is_retryable());
    }

    #[test]
    fn author_errors_are_validation_and_not_retryable() {
        // Every failure the author can fix by editing the expression must be
        // classified as validation — never internal — and must not invite a
        // retry.
        use nebula_error::ErrorCategory;

        for error in [
            ExpressionError::syntax_error("x"),
            ExpressionError::parse_error("x"),
            ExpressionError::eval_error("x"),
            ExpressionError::type_error("number", "string"),
            ExpressionError::invalid_argument("f", "x"),
            ExpressionError::invalid_date("x"),
            ExpressionError::invalid_json("x"),
            ExpressionError::division_by_zero(),
            ExpressionError::regex_error("x"),
            ExpressionError::index_out_of_bounds(1, 0),
            ExpressionError::step_limit_exceeded(1, 2),
            ExpressionError::depth_exceeded(1, 2),
            ExpressionError::NumericOverflow { operation: "add" },
            ExpressionError::NonFiniteNumber { operation: "add" },
        ] {
            assert_eq!(
                error.category(),
                ErrorCategory::Validation,
                "{error:?} must be validation"
            );
            assert!(!error.is_retryable(), "{error:?} must not be retryable");
        }
    }

    #[test]
    fn lookup_errors_are_not_found_and_distinct_from_policy_denial() {
        use nebula_error::ErrorCategory;

        for error in [
            ExpressionError::variable_not_found("x"),
            ExpressionError::function_not_found("f"),
            ExpressionError::property_not_found("p"),
            ExpressionError::key_not_found(),
        ] {
            assert_eq!(error.category(), ErrorCategory::NotFound, "{error:?}");
        }
        assert_eq!(
            ExpressionError::function_not_allowed("f").category(),
            ErrorCategory::Authorization
        );
    }

    #[test]
    fn key_not_found_carries_no_runtime_key() {
        // The redaction invariant is structural: `KeyNotFound` is a unit
        // variant, so there is no field a runtime key could ever occupy.
        // This test pins the observable half — the diagnostic text.
        let error = ExpressionError::key_not_found();
        assert_eq!(error.to_string(), "Object key not found");
        assert_eq!(format!("{error:?}"), "KeyNotFound");
    }

    #[test]
    fn step_limit_variant_carries_limit_and_actual() {
        // Downstream callers must be able to pattern-match StepLimitExceeded
        // and read both numbers — that's the whole point of the typed variant.
        let err = ExpressionError::step_limit_exceeded(100, 105);
        match err {
            ExpressionError::StepLimitExceeded { limit, actual } => {
                assert_eq!(limit, 100);
                assert_eq!(actual, 105);
            },
            other => panic!("expected StepLimitExceeded, got {other:?}"),
        }
    }

    #[test]
    fn depth_exceeded_variant_carries_limit_and_actual() {
        let err = ExpressionError::depth_exceeded(256, 256);
        match err {
            ExpressionError::DepthExceeded { limit, actual } => {
                assert_eq!(limit, 256);
                assert_eq!(actual, 256);
            },
            other => panic!("expected DepthExceeded, got {other:?}"),
        }
    }

    #[test]
    fn step_limit_and_depth_have_distinct_codes() {
        // Classify codes are the contract for routing in upstream error
        // pipelines — they must not collide with each other or with EVAL.
        assert_eq!(
            ExpressionError::step_limit_exceeded(1, 2).code(),
            "EXPR:STEP_LIMIT"
        );
        assert_eq!(
            ExpressionError::depth_exceeded(1, 2).code(),
            "EXPR:DEPTH_LIMIT"
        );
    }
}
