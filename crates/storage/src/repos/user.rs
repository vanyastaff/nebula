//! Identity-layer repositories.

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{PersonalAccessTokenRow, SessionRow, UserRow},
};

/// User account storage.
pub trait UserRepo: Send + Sync {
    /// Insert a new user. Fails if email already exists among active users.
    fn create(&self, user: &UserRow) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a user by ID. Returns `None` if not found or soft-deleted.
    fn get(&self, id: &[u8]) -> impl Future<Output = Result<Option<UserRow>, StorageError>> + Send;

    /// Fetch a user by email (case-insensitive).
    fn get_by_email(
        &self,
        email: &str,
    ) -> impl Future<Output = Result<Option<UserRow>, StorageError>> + Send;

    /// Update a user with CAS on `version`.
    fn update(
        &self,
        user: &UserRow,
        expected_version: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Soft-delete a user (sets `deleted_at`).
    fn soft_delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Record a successful login (updates `last_login_at`, resets failed count).
    fn record_login_success(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Record a failed login attempt. May set `locked_until` after threshold.
    fn record_login_failure(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;
}

/// Session storage for browser logins.
pub trait SessionRepo: Send + Sync {
    /// Insert a new session.
    fn create(&self, session: &SessionRow)
    -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a session by ID. Returns `None` if not found, revoked, or expired.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<SessionRow>, StorageError>> + Send;

    /// Touch `last_active_at` to now.
    fn touch(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Mark the session as revoked.
    fn revoke(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Delete all expired sessions. Returns the count deleted.
    fn cleanup_expired(&self) -> impl Future<Output = Result<u64, StorageError>> + Send;
}

/// Personal access token storage.
pub trait PatRepo: Send + Sync {
    /// Insert a new PAT.
    fn create(
        &self,
        pat: &PersonalAccessTokenRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Look up a PAT by its SHA-256 hash. Returns `None` if not found or revoked.
    fn get_by_hash(
        &self,
        hash: &[u8],
    ) -> impl Future<Output = Result<Option<PersonalAccessTokenRow>, StorageError>> + Send;

    /// Touch `last_used_at` after a successful auth.
    fn touch(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Revoke a PAT (sets `revoked_at`).
    fn revoke(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List active PATs for a principal.
    fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> impl Future<Output = Result<Vec<PersonalAccessTokenRow>, StorageError>> + Send;
}
