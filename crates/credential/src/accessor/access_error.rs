//! Error type for credential access operations.
//!
//! [`CredentialAccessError`] is the error returned by
//! [`CredentialAccessor`](crate::CredentialAccessor) methods. It covers lookup failures, type
//! mismatches, sandbox violations, and missing configuration.

/// Error type for credential access operations.
///
/// Returned by [`CredentialAccessor`](crate::CredentialAccessor) trait methods.
/// Each variant represents a distinct failure mode during credential access.
///
/// # Examples
///
/// ```
/// use nebula_credential::CredentialAccessError;
///
/// let err = CredentialAccessError::NotFound("api_key".to_owned());
/// assert!(err.to_string().contains("api_key"));
/// ```
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialAccessError {
    /// Credential not found.
    #[error("credential not found: {0}")]
    NotFound(String),

    /// Credential type mismatch (scheme projection failed).
    #[error("credential type mismatch: {0}")]
    TypeMismatch(String),

    /// Access to undeclared credential type (sandbox violation).
    #[error("credential access denied: {capability} for action `{action_id}`")]
    AccessDenied {
        /// The capability that was denied.
        capability: String,
        /// The action that requested the capability.
        action_id: String,
    },

    /// Accessor not configured.
    #[error("credential accessor not configured: {0}")]
    NotConfigured(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_display() {
        let err = CredentialAccessError::NotFound("api_key".to_owned());
        assert_eq!(err.to_string(), "credential not found: api_key");
    }

    #[test]
    fn type_mismatch_display() {
        let err = CredentialAccessError::TypeMismatch("expected SecretToken".to_owned());
        assert!(err.to_string().contains("SecretToken"));
    }

    #[test]
    fn access_denied_display() {
        let err = CredentialAccessError::AccessDenied {
            capability: "credential type `OAuth2Token`".to_owned(),
            action_id: "my_action".to_owned(),
        };
        assert!(err.to_string().contains("OAuth2Token"));
        assert!(err.to_string().contains("my_action"));
    }

    #[test]
    fn not_configured_display() {
        let err = CredentialAccessError::NotConfigured(
            "credential capability is not configured".to_owned(),
        );
        assert!(err.to_string().contains("not configured"));
    }

    #[test]
    fn error_is_clone() {
        let err = CredentialAccessError::NotFound("x".to_owned());
        let cloned = err.clone();
        assert_eq!(err.to_string(), cloned.to_string());
    }
}
