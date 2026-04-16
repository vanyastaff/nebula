//! Identity-layer repositories.

use async_trait::async_trait;

use crate::{
    error::StorageError,
    rows::{PersonalAccessTokenRow, SessionRow, UserRow},
};

/// User account storage.
#[async_trait]
pub trait UserRepo: Send + Sync {
    /// Insert a new user. Fails if email already exists among active users.
    async fn create(&self, user: &UserRow) -> Result<(), StorageError>;

    /// Fetch a user by ID. Returns `None` if not found or soft-deleted.
    async fn get(&self, id: &[u8]) -> Result<Option<UserRow>, StorageError>;

    /// Fetch a user by email (case-insensitive).
    async fn get_by_email(&self, email: &str) -> Result<Option<UserRow>, StorageError>;

    /// Update a user with CAS on `version`.
    async fn update(&self, user: &UserRow, expected_version: i64) -> Result<(), StorageError>;

    /// Soft-delete a user (sets `deleted_at`).
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Record a successful login (updates `last_login_at`, resets failed count).
    async fn record_login_success(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Record a failed login attempt. May set `locked_until` after threshold.
    async fn record_login_failure(&self, id: &[u8]) -> Result<(), StorageError>;
}

/// Session storage for browser logins.
#[async_trait]
pub trait SessionRepo: Send + Sync {
    /// Insert a new session.
    async fn create(&self, session: &SessionRow) -> Result<(), StorageError>;

    /// Fetch a session by ID. Returns `None` if not found, revoked, or expired.
    async fn get(&self, id: &[u8]) -> Result<Option<SessionRow>, StorageError>;

    /// Touch `last_active_at` to now.
    async fn touch(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Mark the session as revoked.
    async fn revoke(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Delete all expired sessions. Returns the count deleted.
    async fn cleanup_expired(&self) -> Result<u64, StorageError>;
}

/// Personal access token storage.
#[async_trait]
pub trait PatRepo: Send + Sync {
    /// Insert a new PAT.
    async fn create(&self, pat: &PersonalAccessTokenRow) -> Result<(), StorageError>;

    /// Look up a PAT by its SHA-256 hash. Returns `None` if not found or revoked.
    async fn get_by_hash(
        &self,
        hash: &[u8],
    ) -> Result<Option<PersonalAccessTokenRow>, StorageError>;

    /// Touch `last_used_at` after a successful auth.
    async fn touch(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Revoke a PAT (sets `revoked_at`).
    async fn revoke(&self, id: &[u8]) -> Result<(), StorageError>;

    /// List active PATs for a principal.
    async fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Vec<PersonalAccessTokenRow>, StorageError>;
}
