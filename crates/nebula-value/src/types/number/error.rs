//! Error types for Number operations

use thiserror::Error;

/// Result type alias for number operations
pub type NumberResult<T> = Result<T, NumberError>;

/// Rich, typed errors for number operations
#[derive(Error, Debug, Clone, PartialEq)]
pub enum NumberError {
    #[error("Integer overflow occurred")]
    Overflow,

    #[error("Integer underflow occurred")]
    Underflow,

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Value is not finite (NaN or ±∞)")]
    NotFinite,

    #[error("Value {value} is out of range [{min}, {max}]")]
    OutOfRange {
        value: String,
        min: String,
        max: String,
    },

    #[error("Failed to parse '{input}' as {ty}")]
    ParseError { input: String, ty: &'static str },

    #[error("Loss of precision converting from {from} to {to}")]
    PrecisionLoss {
        from: &'static str,
        to: &'static str,
    },
}