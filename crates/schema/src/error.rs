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

    /// Referenced field does not exist in schema.
    #[error("field not found: {0}")]
    FieldNotFound(String),

    /// Field exists but has an unexpected type for the requested operation.
    #[error("field `{key}` has invalid type: expected {expected}, got {actual}")]
    InvalidFieldType {
        /// Referenced field key.
        key: String,
        /// Expected field type name.
        expected: &'static str,
        /// Actual field type name.
        actual: &'static str,
    },

    /// Field is dynamic but no loader key was configured.
    #[error("field `{0}` has no loader configured")]
    LoaderNotConfigured(String),

    /// Rule validation failure from the validator crate.
    #[error("validation failed: {0}")]
    Validation(#[from] nebula_validator::ValidatorError),

    /// Runtime loader invocation failed.
    #[error("loader failed: {0}")]
    Loader(#[from] crate::loader::LoaderError),
}
