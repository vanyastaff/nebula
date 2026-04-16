use thiserror::Error;

/// Errors raised by schema construction and validation.
#[derive(Debug, Error)]
pub enum SchemaError {
    /// Field key violates format rules.
    #[error("invalid field key: {0}")]
    InvalidKey(String),

    /// Duplicate key detected in a single schema.
    #[error("duplicate field key: {0}")]
    DuplicateKey(String),

    /// Rule validation failure from the validator crate.
    #[error("validation failed: {0}")]
    Validation(#[from] nebula_validator::ValidatorError),
}
