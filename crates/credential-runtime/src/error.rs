//! Error taxonomy for the credential management runtime.
//!
//! `#[non_exhaustive]` so later increments add variants without breaking
//! downstream `match` exhaustiveness. Classified via
//! [`nebula_error::Classify`] using only the codebase-standard categories
//! `internal` / `validation` / `external` (mirrors
//! `crates/credential/src/error.rs`).

use thiserror::Error;

/// Failure modes of the credential management facade. The API layer maps
/// each `category` to an HTTP status; `code` is the stable machine label.
#[derive(Debug, Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum CredentialServiceError {
    /// No credential with this id in the caller's tenant scope.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:NOT_FOUND")]
    #[error("credential not found: {id}")]
    NotFound {
        /// The credential id that was not found.
        id: String,
    },

    /// Optimistic-concurrency check failed on update.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:VERSION_CONFLICT")]
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict {
        /// Credential id under contention.
        id: String,
        /// Version the caller expected (CAS precondition).
        expected: u64,
        /// Version actually stored.
        actual: u64,
    },

    /// Property payload failed the credential type's schema validation.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:VALIDATION_FAILED")]
    #[error("credential property validation failed: {reason}")]
    ValidationFailed {
        /// Human-readable validation failure (never echoes secret values).
        reason: String,
    },

    /// The requested lifecycle op needs a capability the type lacks.
    #[classify(
        category = "validation",
        code = "CREDENTIAL_SERVICE:CAPABILITY_UNSUPPORTED"
    )]
    #[error("credential type '{key}' does not support capability '{capability}'")]
    CapabilityUnsupported {
        /// Capability name (`refresh` / `revoke` / `test`).
        capability: String,
        /// `Credential::KEY` of the target type.
        key: String,
    },

    /// No credential type registered under this key.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:TYPE_UNKNOWN")]
    #[error("unknown credential type: {key}")]
    TypeUnknown {
        /// The unregistered credential key.
        key: String,
    },

    /// Interactive acquisition token is expired or already consumed.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:PENDING_EXPIRED")]
    #[error("pending acquisition expired or already consumed")]
    PendingExpired,

    /// An external secret provider failed.
    #[classify(category = "external", code = "CREDENTIAL_SERVICE:PROVIDER")]
    #[error("external provider error: {0}")]
    Provider(String),

    /// The persistence layer failed.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:STORE")]
    #[error("credential store error: {0}")]
    Store(String),

    /// An invariant the runtime owns was violated.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:INTERNAL")]
    #[error("internal credential runtime error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::CredentialServiceError;

    #[test]
    fn display_messages_are_actionable() {
        let e = CredentialServiceError::NotFound {
            id: "cred-1".to_owned(),
        };
        assert_eq!(e.to_string(), "credential not found: cred-1");

        let e = CredentialServiceError::VersionConflict {
            id: "cred-1".to_owned(),
            expected: 3,
            actual: 4,
        };
        assert_eq!(
            e.to_string(),
            "version conflict for cred-1: expected 3, got 4"
        );

        let e = CredentialServiceError::CapabilityUnsupported {
            capability: "refresh".to_owned(),
            key: "bearer_token".to_owned(),
        };
        assert_eq!(
            e.to_string(),
            "credential type 'bearer_token' does not support capability 'refresh'"
        );
    }

    #[test]
    fn is_std_error() {
        fn assert_error<E: std::error::Error + Send + Sync + 'static>() {}
        assert_error::<CredentialServiceError>();
    }
}
