use thiserror::Error;
use crate::types::{Key, KeyParseError};

#[derive(Debug, Error)]
pub enum ActionError {
    /// Action identified by `key` was not found.
    #[error("Action '{0}' is not found")]
    NotFound(Key),

    /// Build error (e.g., during configuration struct building).
    #[error("Build error: {0}")]
    BuildError(#[from] derive_builder::UninitializedFieldError),

    /// Invalid format or content for a parameter key string.
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(#[from] KeyParseError),
    
    /// Error during action execution.
    #[error("Action execution error: {message}")]
    Execution { message: String },
    
    /// Cancelled action.
    #[error("Action was canceled")]
    Cancelled,
    
    /// Action execution timed out.
    #[error("Action execution timed out after {timeout_ms} ms")]
    Timeout { timeout_ms: u64 },

    /// Unsupported operation for the action.
    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(String),
}
