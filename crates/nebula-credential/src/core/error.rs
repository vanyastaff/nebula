use thiserror::Error;

/// Main error type for credential operations
#[derive(Error, Debug, Clone)]
pub enum CredentialError {
    /// Credential not found
    #[error("Credential not found: {id}")]
    NotFound {
        /// The credential ID
        id: String,
    },

    /// Credential has expired and cannot be refreshed
    #[error("Credential expired: {id}")]
    Expired {
        /// The credential ID
        id: String,
    },

    /// Invalid key format error
    #[error("Credential invalid key: {0}")]
    InvalidKey(#[from] domain_key::KeyParseError),

    /// Refresh operation is not supported for this credential type
    #[error("Refresh not supported for credential type: {credential_type}")]
    RefreshNotSupported {
        /// The credential type
        credential_type: String,
    },

    /// Failed to refresh credential
    #[error("Failed to refresh credential {id}: {reason}")]
    RefreshFailed {
        /// The credential ID
        id: String,
        /// The failure reason
        reason: String,
    },

    /// Authentication failed
    #[error("Authentication failed: {reason}")]
    AuthenticationFailed {
        /// The failure reason
        reason: String,
    },

    /// Invalid credential configuration
    #[error("Invalid credential configuration: {reason}")]
    InvalidConfiguration {
        /// The configuration issue
        reason: String,
    },

    /// Storage operation failed
    #[error("Storage operation failed: {operation}: {reason}")]
    StorageFailed {
        /// The storage operation
        operation: String,
        /// The failure reason
        reason: String,
    },

    /// Cache operation failed
    #[error("Cache operation failed: {operation}: {reason}")]
    CacheFailed {
        /// The cache operation
        operation: String,
        /// The failure reason
        reason: String,
    },

    /// Lock acquisition failed
    #[error("Failed to acquire lock: {resource}: {reason}")]
    LockFailed {
        /// The locked resource
        resource: String,
        /// The failure reason
        reason: String,
    },

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
    Timeout {
        /// The operation that timed out
        operation: String,
    },

    /// Invalid input provided
    #[error("Invalid input: {field}: {reason}")]
    InvalidInput {
        /// The invalid field
        field: String,
        /// The validation error
        reason: String,
    },

    /// Credential type not registered
    #[error("Credential type not registered: {credential_type}")]
    TypeNotRegistered {
        /// The unregistered credential type
        credential_type: String,
    },

    /// Credential already exists
    #[error("Credential already exists: {id}")]
    AlreadyExists {
        /// The credential ID
        id: String,
    },

    /// Permission denied
    #[error("Permission denied: {operation}: {reason}")]
    PermissionDenied {
        /// The denied operation
        operation: String,
        /// The denial reason
        reason: String,
    },

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),

    /// Custom error from credential implementation
    #[error("Credential error: {message}")]
    Custom {
        /// The error message
        message: String,
    },

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_not_found() {
        let err = CredentialError::not_found("test-id");
        assert!(matches!(err, CredentialError::NotFound { .. }));
        assert_eq!(err.to_string(), "Credential not found: test-id");
    }

    #[test]
    fn test_error_expired() {
        let err = CredentialError::expired("expired-id");
        assert!(matches!(err, CredentialError::Expired { .. }));
        assert_eq!(err.to_string(), "Credential expired: expired-id");
    }

    #[test]
    fn test_error_refresh_not_supported() {
        let err = CredentialError::refresh_not_supported("static_key");
        assert!(matches!(err, CredentialError::RefreshNotSupported { .. }));
        assert!(err.to_string().contains("static_key"));
    }

    #[test]
    fn test_error_auth_failed() {
        let err = CredentialError::auth_failed("invalid credentials");
        assert!(matches!(err, CredentialError::AuthenticationFailed { .. }));
        assert!(err.to_string().contains("invalid credentials"));
    }

    #[test]
    fn test_error_storage_failed() {
        let err = CredentialError::storage_failed("save", "connection lost");
        assert!(matches!(err, CredentialError::StorageFailed { .. }));
        assert!(err.to_string().contains("save"));
        assert!(err.to_string().contains("connection lost"));
    }

    #[test]
    fn test_error_invalid_input() {
        let err = CredentialError::invalid_input("username", "cannot be empty");
        assert!(matches!(err, CredentialError::InvalidInput { .. }));
        assert!(err.to_string().contains("username"));
    }

    #[test]
    fn test_error_internal() {
        let err = CredentialError::internal("panic occurred");
        assert!(matches!(err, CredentialError::Internal(_)));
        assert!(err.to_string().contains("panic occurred"));
    }

    #[test]
    fn test_error_needs_refresh() {
        assert!(CredentialError::expired("id").needs_refresh());
        assert!(CredentialError::auth_failed("reason").needs_refresh());
        assert!(!CredentialError::not_found("id").needs_refresh());
        assert!(!CredentialError::internal("msg").needs_refresh());
    }

    #[test]
    fn test_error_category() {
        assert_eq!(CredentialError::not_found("id").category(), "not_found");
        assert_eq!(CredentialError::expired("id").category(), "expired");
        assert_eq!(
            CredentialError::auth_failed("x").category(),
            "authentication_failed"
        );
        assert_eq!(
            CredentialError::storage_failed("op", "x").category(),
            "storage_failed"
        );
        assert_eq!(CredentialError::CasConflict.category(), "cas_conflict");
    }

    #[test]
    fn test_error_is_retryable() {
        assert!(CredentialError::NetworkFailed("timeout".to_string()).is_retryable());
        assert!(CredentialError::Timeout {
            operation: "fetch".to_string()
        }
        .is_retryable());
        assert!(CredentialError::storage_failed("save", "db down").is_retryable());

        assert!(!CredentialError::not_found("id").is_retryable());
        assert!(!CredentialError::expired("id").is_retryable());
        assert!(!CredentialError::auth_failed("bad creds").is_retryable());
    }

    #[test]
    fn test_error_clone() {
        let original = CredentialError::not_found("test-id");
        let cloned = original.clone();

        assert_eq!(original.to_string(), cloned.to_string());
        assert_eq!(original.category(), cloned.category());
    }

    #[test]
    fn test_error_from_serde_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json");
        assert!(json_err.is_err());

        let cred_err: CredentialError = json_err.unwrap_err().into();
        assert!(matches!(
            cred_err,
            CredentialError::DeserializationFailed(_)
        ));
    }

    #[test]
    fn test_error_display_implementation() {
        let err = CredentialError::RefreshFailed {
            id: "cred-123".to_string(),
            reason: "network error".to_string(),
        };
        let display_str = err.to_string();
        assert!(display_str.contains("cred-123"));
        assert!(display_str.contains("network error"));
    }

    #[test]
    fn test_error_cas_conflict() {
        let err = CredentialError::CasConflict;
        assert_eq!(err.category(), "cas_conflict");
        assert!(!err.is_retryable());
        assert!(!err.needs_refresh());
    }
}
