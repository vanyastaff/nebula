//! Credential repository.
//!
//! Spec 22. Stores encrypted credentials; decryption happens exclusively
//! in `nebula-credential`. This trait never returns plaintext.

use async_trait::async_trait;

use crate::{
    error::StorageError,
    rows::{CredentialAuditRow, CredentialRow, PendingCredentialRow},
};

/// Encrypted credential storage with audit.
#[async_trait]
pub trait CredentialRepo: Send + Sync {
    // ── Credentials ─────────────────────────────────────────────────────

    /// Insert a new credential.
    async fn create(&self, cred: &CredentialRow) -> Result<(), StorageError>;

    /// Fetch a credential by ID (skips soft-deleted).
    async fn get(&self, id: &[u8]) -> Result<Option<CredentialRow>, StorageError>;

    /// Fetch a credential by (workspace, slug) or (org, slug).
    async fn get_by_slug(
        &self,
        scope: CredentialScope<'_>,
        slug: &str,
    ) -> Result<Option<CredentialRow>, StorageError>;

    /// Update a credential with CAS on `version` (rotates ciphertext).
    async fn update(&self, cred: &CredentialRow, expected_version: i64)
    -> Result<(), StorageError>;

    /// Record a use (`last_used_at`).
    async fn touch_used(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Soft-delete a credential.
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    /// List credentials visible in a scope.
    async fn list(&self, scope: CredentialScope<'_>) -> Result<Vec<CredentialRow>, StorageError>;

    // ── Pending (OAuth flow) credentials ────────────────────────────────

    /// Stage a pending credential (mid-OAuth flow).
    async fn create_pending(&self, pending: &PendingCredentialRow) -> Result<(), StorageError>;

    /// Fetch a pending credential (skips expired).
    async fn get_pending(&self, id: &[u8]) -> Result<Option<PendingCredentialRow>, StorageError>;

    /// Delete a pending credential.
    async fn delete_pending(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Clean up expired pending credentials. Returns count deleted.
    async fn cleanup_expired_pending(&self) -> Result<u64, StorageError>;

    // ── Audit ───────────────────────────────────────────────────────────

    /// Append an audit row for a credential action.
    async fn append_audit(&self, row: &CredentialAuditRow) -> Result<(), StorageError>;

    /// List audit rows for a credential (newest first).
    async fn list_audit(
        &self,
        credential_id: &[u8],
        limit: u32,
    ) -> Result<Vec<CredentialAuditRow>, StorageError>;
}

/// Scope of a credential query.
#[derive(Debug, Clone, Copy)]
pub enum CredentialScope<'a> {
    /// Credentials owned by a workspace.
    Workspace(&'a [u8]),
    /// Credentials owned by an org.
    Org(&'a [u8]),
}
