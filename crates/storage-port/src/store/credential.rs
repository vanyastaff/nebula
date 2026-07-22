//! Directly object-safe credential persistence port.

use std::{fmt, sync::Arc};

use async_trait::async_trait;

use crate::dto::credential::{
    CredentialOwner, CredentialSelector, CredentialWriteMode, StoredCredential,
    StoredCredentialHead,
};

/// Credential persistence failure.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum CredentialPersistenceError {
    /// No row exists for the complete owner-bound selector.
    #[error("credential not found")]
    NotFound {
        /// Opaque credential id.
        credential_id: String,
    },
    /// Compare-and-swap observed a different version.
    #[error("credential version conflict: expected {expected}, got {actual}")]
    VersionConflict {
        /// Opaque credential id.
        credential_id: String,
        /// Expected version.
        expected: u64,
        /// Actual persisted version.
        actual: u64,
    },
    /// Create-only write collided with an existing row.
    #[error("credential already exists")]
    AlreadyExists {
        /// Opaque credential id.
        credential_id: String,
    },
    /// Audit recording failed. The mutation may already be committed because
    /// the interim sink seam is not transactional with persistence.
    #[error("credential audit sink refused the operation")]
    AuditFailure(String),
    /// Caller supplied structurally inconsistent port data.
    #[error("invalid credential persistence request: {0}")]
    InvalidRequest(&'static str),
    /// Backend failure.
    #[error("credential persistence backend failed")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Debug for CredentialPersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { .. } => formatter
                .debug_struct("NotFound")
                .field("credential_id", &"[redacted]")
                .finish(),
            Self::VersionConflict {
                expected, actual, ..
            } => formatter
                .debug_struct("VersionConflict")
                .field("credential_id", &"[redacted]")
                .field("expected", expected)
                .field("actual", actual)
                .finish(),
            Self::AlreadyExists { .. } => formatter
                .debug_struct("AlreadyExists")
                .field("credential_id", &"[redacted]")
                .finish(),
            Self::AuditFailure(_) => formatter.write_str("AuditFailure([redacted])"),
            Self::InvalidRequest(message) => formatter
                .debug_tuple("InvalidRequest")
                .field(message)
                .finish(),
            Self::Backend(_) => formatter.write_str("Backend([redacted])"),
        }
    }
}

/// Owner-scoped credential persistence.
///
/// The trait is directly object-safe; consumers store `Arc<dyn
/// CredentialPersistence>` without a parallel RPITIT bridge. Every per-row
/// operation receives one owner-bound selector, and list receives a mandatory
/// owner. Adapters must never interpret an absent or empty owner as global
/// access.
#[async_trait]
pub trait CredentialPersistence: Send + Sync + fmt::Debug {
    /// Load one row by its complete owner-bound selector.
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError>;

    /// Load one secret-free projection without fetching state bytes.
    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError>;

    /// Insert or replace one row under its complete owner-bound selector.
    async fn put(
        &self,
        selector: &CredentialSelector,
        credential: StoredCredential,
        mode: CredentialWriteMode,
    ) -> Result<StoredCredential, CredentialPersistenceError>;

    /// Delete one row by its complete owner-bound selector.
    async fn delete(&self, selector: &CredentialSelector)
    -> Result<(), CredentialPersistenceError>;

    /// List ids in exactly one owner partition, optionally filtered by state kind.
    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<String>, CredentialPersistenceError>;

    /// List secret-free projections in exactly one owner partition.
    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError>;

    /// Test existence under the complete owner-bound selector.
    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError>;
}

#[async_trait]
impl<T> CredentialPersistence for Arc<T>
where
    T: CredentialPersistence + ?Sized,
{
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        (**self).get(selector).await
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        (**self).get_head(selector).await
    }

    async fn put(
        &self,
        selector: &CredentialSelector,
        credential: StoredCredential,
        mode: CredentialWriteMode,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        (**self).put(selector, credential, mode).await
    }

    async fn delete(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(), CredentialPersistenceError> {
        (**self).delete(selector).await
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<String>, CredentialPersistenceError> {
        (**self).list(owner, state_kind).await
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        (**self).list_heads(owner, state_kind).await
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        (**self).exists(selector).await
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialPersistenceError;

    #[test]
    fn dynamic_failure_details_never_render() {
        const CANARY: &str = "credential-persistence-error-secret-73af";
        let failures = [
            CredentialPersistenceError::AuditFailure(CANARY.to_owned()),
            CredentialPersistenceError::Backend(Box::new(std::io::Error::other(CANARY))),
            CredentialPersistenceError::NotFound {
                credential_id: CANARY.to_owned(),
            },
            CredentialPersistenceError::AlreadyExists {
                credential_id: CANARY.to_owned(),
            },
            CredentialPersistenceError::VersionConflict {
                credential_id: CANARY.to_owned(),
                expected: 1,
                actual: 2,
            },
        ];

        for failure in failures {
            assert!(!failure.to_string().contains(CANARY));
            assert!(!format!("{failure:?}").contains(CANARY));
        }
    }
}
