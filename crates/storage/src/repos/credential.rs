//! Credential repository.
//!
//! Spec 22. Stores encrypted credentials; decryption happens exclusively
//! in `nebula-credential`. This trait never returns plaintext.

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{CredentialAuditRow, CredentialRow, PendingCredentialRow},
};

/// Encrypted credential storage with audit.
pub trait CredentialRepo: Send + Sync {
    // ── Credentials ─────────────────────────────────────────────────────

    /// Insert a new credential.
    fn create(&self, cred: &CredentialRow)
    -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a credential by ID (skips soft-deleted).
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<CredentialRow>, StorageError>> + Send;

    /// Fetch a credential by (workspace, slug) or (org, slug).
    fn get_by_slug(
        &self,
        scope: CredentialScope<'_>,
        slug: &str,
    ) -> impl Future<Output = Result<Option<CredentialRow>, StorageError>> + Send;

    /// Update a credential with CAS on `version` (rotates ciphertext).
    fn update(
        &self,
        cred: &CredentialRow,
        expected_version: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Record a use (`last_used_at`).
    fn touch_used(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Soft-delete a credential.
    fn soft_delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List credentials visible in a scope.
    fn list(
        &self,
        scope: CredentialScope<'_>,
    ) -> impl Future<Output = Result<Vec<CredentialRow>, StorageError>> + Send;

    // ── Pending (OAuth flow) credentials ────────────────────────────────

    /// Stage a pending credential (mid-OAuth flow).
    fn create_pending(
        &self,
        pending: &PendingCredentialRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a pending credential (skips expired).
    fn get_pending(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<PendingCredentialRow>, StorageError>> + Send;

    /// Delete a pending credential.
    fn delete_pending(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Clean up expired pending credentials. Returns count deleted.
    fn cleanup_expired_pending(&self) -> impl Future<Output = Result<u64, StorageError>> + Send;

    // ── Audit ───────────────────────────────────────────────────────────

    /// Append an audit row for a credential action.
    fn append_audit(
        &self,
        row: &CredentialAuditRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List audit rows for a credential (newest first).
    fn list_audit(
        &self,
        credential_id: &[u8],
        limit: u32,
    ) -> impl Future<Output = Result<Vec<CredentialAuditRow>, StorageError>> + Send;
}

/// Scope of a credential query.
#[derive(Debug, Clone, Copy)]
pub enum CredentialScope<'a> {
    /// Credentials owned by a workspace.
    Workspace(&'a [u8]),
    /// Credentials owned by an org.
    Org(&'a [u8]),
}
