use thiserror::Error;

/// Main error type for credential operations
#[derive(Error, Debug, Clone)]
pub enum CredentialError {
    /// Credential not found
    #[error("Credential not found: {id}")]
    NotFound { id: String },

    /// Credential has expired and cannot be refreshed
    #[error("Credential expired: {id}")]
    Expired { id: String },

    #[error("Credential invalid key: {0}")]
    InvalidKey(#[from] domain_key::KeyParseError),

    /// Refresh operation is not supported for this credential type
    #[error("Refresh not supported for credential type: {credential_type}")]
    RefreshNotSupported { credential_type: String },

    /// Failed to refresh credential
    #[error("Failed to refresh credential {id}: {reason}")]
    RefreshFailed { id: String, reason: String },

    /// Authentication failed
    #[error("Authentication failed: {reason}")]
    AuthenticationFailed { reason: String },

    /// Invalid credential configuration
    #[error("Invalid credential configuration: {reason}")]
    InvalidConfiguration { reason: String },

    /// Storage operation failed
    #[error("Storage operation failed: {operation}: {reason}")]
    StorageFailed { operation: String, reason: String },

    /// Cache operation failed
    #[error("Cache operation failed: {operation}: {reason}")]
    CacheFailed { operation: String, reason: String },

    /// Lock acquisition failed
    #[error("Failed to acquire lock: {resource}: {reason}")]
    LockFailed { resource: String, reason: String },

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationFailed(String),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationFailed(String),

    /// Network operation failed
    #[error("Network operation failed: {0}")]
    NetworkFailed(String),

    /// Timeout occurred
    #[error("Operation timed out: {operation}")]
    Timeout { operation: String },

    /// Invalid input provided
    #[error("Invalid input: {field}: {reason}")]
    InvalidInput { field: String, reason: String },

    /// Credential type not registered
    #[error("Credential type not registered: {credential_type}")]
    TypeNotRegistered { credential_type: String },

    /// Credential already exists
    #[error("Credential already exists: {id}")]
    AlreadyExists { id: String },

    /// Permission denied
    #[error("Permission denied: {operation}: {reason}")]
    PermissionDenied { operation: String, reason: String },

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),

    /// Custom error from credential implementation
    #[error("Credential error: {message}")]
    Custom { message: String },

    /// Compare-and-swap conflict
    #[error("Compare-and-swap conflict during credential update")]
    CasConflict,
}

impl CredentialError {
    /// Create a new "not found" error
    pub fn not_found(id: impl Into<String>) -> Self {
        Self::NotFound { id: id.into() }
    }

    /// Create a new "expired" error
    pub fn expired(id: impl Into<String>) -> Self {
        Self::Expired { id: id.into() }
    }

    /// Create a new "refresh not supported" error
    pub fn refresh_not_supported(credential_type: impl Into<String>) -> Self {
        Self::RefreshNotSupported {
            credential_type: credential_type.into(),
        }
    }

    /// Create a new "authentication failed" error
    pub fn auth_failed(reason: impl Into<String>) -> Self {
        Self::AuthenticationFailed {
            reason: reason.into(),
        }
    }

    /// Create a new "storage failed" error
    pub fn storage_failed(operation: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::StorageFailed {
            operation: operation.into(),
            reason: reason.into(),
        }
    }

    /// Create a new "invalid input" error
    pub fn invalid_input(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidInput {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// Create a new "internal" error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// Check if this error indicates the credential needs refresh
    pub fn needs_refresh(&self) -> bool {
        matches!(
            self,
            Self::Expired { .. } | Self::AuthenticationFailed { .. }
        )
    }

    /// Get the error category for logging/metrics
    pub fn category(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not_found",
            Self::Expired { .. } => "expired",
            Self::InvalidKey(_) => "invalid_key",
            Self::RefreshNotSupported { .. } => "refresh_not_supported",
            Self::RefreshFailed { .. } => "refresh_failed",
            Self::AuthenticationFailed { .. } => "authentication_failed",
            Self::InvalidConfiguration { .. } => "invalid_configuration",
            Self::StorageFailed { .. } => "storage_failed",
            Self::CacheFailed { .. } => "cache_failed",
            Self::LockFailed { .. } => "lock_failed",
            Self::SerializationFailed(_) => "serialization_failed",
            Self::DeserializationFailed(_) => "deserialization_failed",
            Self::NetworkFailed(_) => "network_failed",
            Self::Timeout { .. } => "timeout",
            Self::InvalidInput { .. } => "invalid_input",
            Self::TypeNotRegistered { .. } => "type_not_registered",
            Self::AlreadyExists { .. } => "already_exists",
            Self::PermissionDenied { .. } => "permission_denied",
            Self::Internal(_) => "internal",
            Self::Custom { .. } => "custom",
            Self::CasConflict => "cas_conflict",
        }
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::NetworkFailed(_)
                | Self::Timeout { .. }
                | Self::StorageFailed { .. }
                | Self::CacheFailed { .. }
                | Self::LockFailed { .. }
        )
    }
}

/// Result type alias for credential operations
pub type Result<T> = std::result::Result<T, CredentialError>;

/// Convert from `serde_json` errors
impl From<serde_json::Error> for CredentialError {
    fn from(error: serde_json::Error) -> Self {
        if error.is_syntax() || error.is_data() {
            Self::DeserializationFailed(error.to_string())
        } else {
            Self::SerializationFailed(error.to_string())
        }
    }
}
