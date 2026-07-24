//! Directly object-safe credential persistence port.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use nebula_core::CredentialId;

use crate::dto::RefreshRetrySnapshot;
use crate::dto::credential::{
    CredentialCommit, CredentialCreate, CredentialOwner, CredentialReplacement, CredentialSelector,
    CredentialTombstone, CredentialVersion, StoredCredential, StoredCredentialHead,
};

/// Unique credential field that rejected a create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CredentialAlreadyExistsKey {
    /// The globally unique credential id is permanently reserved.
    Id,
    /// A live credential already owns the owner-local display name.
    Name,
}

/// Closed credential persistence failure.
///
/// Variants contain only bounded typed context. Driver messages, SQL,
/// database locations, owner keys, credential ids, names, and secret material
/// never cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialPersistenceError {
    /// No admissible live row exists for the complete owner-bound selector.
    #[error("credential not found")]
    NotFound,

    /// Compare-and-swap observed a different live version.
    #[error("credential version conflict: expected {expected}, got {actual}")]
    VersionConflict {
        /// Version supplied by the command.
        expected: CredentialVersion,
        /// Version stored by the backend.
        actual: CredentialVersion,
    },

    /// Create collided with a permanently reserved id or live name.
    #[error("credential already exists")]
    AlreadyExists {
        /// Typed uniqueness boundary that collided.
        key: CredentialAlreadyExistsKey,
    },

    /// The requested transition cannot preserve the version bound.
    #[error("credential version exhausted")]
    VersionExhausted,

    /// The requested material-authority transition cannot advance its epoch.
    #[error("credential material epoch exhausted")]
    MaterialEpochExhausted,

    /// A persisted row violates the closed credential record contract.
    #[error("credential record is corrupt")]
    CorruptRecord,

    /// The operation definitely did not commit.
    #[error("credential persistence is unavailable")]
    Unavailable,

    /// Commit was dispatched but authoritative acknowledgement was lost.
    #[error("credential persistence outcome is unknown; do not retry blindly")]
    OutcomeUnknown,
}

/// Owner-scoped credential persistence.
///
/// The trait is directly object-safe; consumers store `Arc<dyn
/// CredentialPersistence>`. Every row operation receives a mandatory
/// owner-bound selector. Management projections and existence checks are
/// live-only; [`Self::get`] deliberately exposes a physical tombstone to
/// trusted binding logic.
#[async_trait]
pub trait CredentialPersistence: Send + Sync + fmt::Debug {
    /// Load one physical live or terminal record.
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError>;

    /// Load one secret-free live projection without fetching state bytes.
    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError>;

    /// Atomically observe live version, material epoch, reauthentication, and
    /// retry admission.
    ///
    /// The complete result comes from one backend snapshot and the admission
    /// is evaluated against the clock sampled by that same read. Missing,
    /// wrong-owner, and tombstoned rows return
    /// [`CredentialPersistenceError::NotFound`]; malformed gates fail closed
    /// as [`CredentialPersistenceError::CorruptRecord`].
    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError>;

    /// Insert one new live credential.
    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError>;

    /// Replace mutable state of one version-matched live credential.
    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError>;

    /// Transition one version-matched live credential to terminal state.
    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError>;

    /// List typed ids of live credentials in exactly one owner partition.
    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError>;

    /// List secret-free live projections in exactly one owner partition.
    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError>;

    /// Test live-record existence under the complete owner-bound selector.
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

    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        (**self).refresh_retry_snapshot(selector).await
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        (**self).create(selector, create).await
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        (**self).replace(selector, replacement).await
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        (**self).tombstone(selector, tombstone).await
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
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
    use super::{CredentialAlreadyExistsKey, CredentialPersistenceError};
    use crate::CredentialVersion;

    #[test]
    fn errors_have_closed_secret_free_diagnostics() {
        let one = CredentialVersion::MIN;
        let two = one.next_live().expect("one can advance");
        let failures = [
            CredentialPersistenceError::NotFound,
            CredentialPersistenceError::VersionConflict {
                expected: one,
                actual: two,
            },
            CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Id,
            },
            CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Name,
            },
            CredentialPersistenceError::VersionExhausted,
            CredentialPersistenceError::MaterialEpochExhausted,
            CredentialPersistenceError::CorruptRecord,
            CredentialPersistenceError::Unavailable,
            CredentialPersistenceError::OutcomeUnknown,
        ];

        for failure in failures {
            let rendered = format!("{failure} {failure:?}");
            for canary in [
                "credential-secret-canary",
                "tenant-canary",
                "cred_",
                "postgres://",
                "SELECT ",
            ] {
                assert!(!rendered.contains(canary));
            }
        }
    }
}
