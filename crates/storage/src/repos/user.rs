//! Identity-layer repositories.

use std::future::Future;

use crate::{
    error::StorageError,
    rows::{OAuthStateRow, PersonalAccessTokenRow, SessionRow, UserRow, VerificationTokenRow},
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

/// One-time verification tokens (email verification, password reset,
/// MFA challenges, invitations).
///
/// Tokens are stored by SHA-256 hash of the plaintext value; the
/// plaintext is only available to the caller at mint time and is sent
/// to the user out-of-band (email link, etc.).
pub trait VerificationTokenRepo: Send + Sync {
    /// Insert a new verification token.
    fn create(
        &self,
        token: &VerificationTokenRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Atomically mark a token as consumed and return its row. Returns
    /// `None` if the token does not exist, is already consumed, or has
    /// expired.
    ///
    /// Prefer [`consume_by_hash_and_kind`](Self::consume_by_hash_and_kind)
    /// for routes that only accept a specific `kind` (e.g. MFA
    /// challenge) — that variant filters on `kind` inside the same SQL
    /// statement, so a token of the wrong kind sent to the wrong
    /// endpoint is rejected as `None` without being burned.
    fn consume_by_hash(
        &self,
        token_hash: &[u8],
    ) -> impl Future<Output = Result<Option<VerificationTokenRow>, StorageError>> + Send;

    /// Atomically mark a token as consumed **only when both `token_hash`
    /// AND `kind` match** an unconsumed, unexpired row, and return that
    /// row. Returns `None` for any mismatch — including a valid token
    /// presented to the wrong route (where `kind` differs) — so a
    /// password-reset token sent to the MFA-verify endpoint cannot be
    /// destroyed by a blind consume.
    fn consume_by_hash_and_kind(
        &self,
        token_hash: &[u8],
        kind: &str,
    ) -> impl Future<Output = Result<Option<VerificationTokenRow>, StorageError>> + Send;

    /// Fetch a token by hash without consuming it. Returns `None` if not
    /// found. Caller is responsible for checking `expires_at` /
    /// `consumed_at`. Primarily a test helper.
    fn get_by_hash(
        &self,
        token_hash: &[u8],
    ) -> impl Future<Output = Result<Option<VerificationTokenRow>, StorageError>> + Send;

    /// Delete all expired (`expires_at < now`) tokens. Returns the
    /// count deleted.
    fn cleanup_expired(&self) -> impl Future<Output = Result<u64, StorageError>> + Send;

    /// Mark all unconsumed tokens for a user of the given `kind` as
    /// consumed. Used to invalidate in-flight reset / verification
    /// links after a successful action (e.g. password change). Returns
    /// the count revoked.
    fn revoke_all_for_user(
        &self,
        user_id: &[u8],
        kind: &str,
    ) -> impl Future<Output = Result<u64, StorageError>> + Send;
}

/// Server-side storage for Plane-A OAuth PKCE state.
///
/// Each `start_oauth` mints a row keyed by the random url-safe state
/// string; the matching `complete_oauth` atomically consumes the row
/// to recover the PKCE `code_verifier` and validate the callback.
/// Distinct from the Plane-B credential OAuth surface, which has its
/// own state-pending table — see `0008_credentials.sql` family.
pub trait OAuthStateRepo: Send + Sync {
    /// Insert a new PKCE state row.
    fn create(
        &self,
        state: &OAuthStateRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Atomically mark a PKCE state as consumed and return its row.
    /// Returns `None` if the state does not exist, is already consumed,
    /// or has expired. The repo MUST NOT return the row twice for the
    /// same state value — this is the replay defence.
    ///
    /// Prefer
    /// [`consume_by_state_and_provider`](Self::consume_by_state_and_provider)
    /// when the caller knows which provider the callback came from —
    /// that variant filters on `provider` inside the same SQL statement,
    /// so a state value crossed between providers is rejected as `None`
    /// without being burned.
    fn consume_by_state(
        &self,
        state: &str,
    ) -> impl Future<Output = Result<Option<OAuthStateRow>, StorageError>> + Send;

    /// Atomically mark a PKCE state as consumed **only when both `state`
    /// AND `provider` match** an unconsumed, unexpired row, and return
    /// that row. Returns `None` on any mismatch (including a state value
    /// crossed between providers), so a callback presenting the wrong
    /// provider cannot destroy a valid row.
    fn consume_by_state_and_provider(
        &self,
        state: &str,
        provider: &str,
    ) -> impl Future<Output = Result<Option<OAuthStateRow>, StorageError>> + Send;

    /// Delete all expired (`expires_at < now`) rows. Returns the count
    /// deleted.
    fn cleanup_expired(&self) -> impl Future<Output = Result<u64, StorageError>> + Send;

    /// Fetch a state row without consuming it. Returns `None` if not
    /// found. Primarily a test helper — production paths must use
    /// [`consume_by_state`](Self::consume_by_state) so the row cannot
    /// be replayed.
    fn get_by_state(
        &self,
        state: &str,
    ) -> impl Future<Output = Result<Option<OAuthStateRow>, StorageError>> + Send;
}

/// Repository for the `external_identities` table (Plane-A OAuth
/// provider ↔ Nebula user linkage). Per ADR-0085 D-8 + REQ-oauth-005
/// / REQ-oauth-006.
///
/// Read path serves the REQ-oauth-006 short-circuit on repeat logins
/// (find_user_by_external returning `Some(user_id)` means the user
/// has logged in via this IdP before; mint session directly without
/// consulting the email truth-table). Write path runs on first login
/// AND on each verified-email cross-link (REQ-oauth-004 / -005).
pub trait ExternalIdentityRepo: Send + Sync {
    /// Resolve `(provider, subject)` to a Nebula `user_id`. Returns
    /// `None` when there is no existing link — the caller then falls
    /// through to the email truth-table (first login or existing-user
    /// link by verified email).
    fn find_user_by_external(
        &self,
        provider: &str,
        subject: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StorageError>> + Send;

    /// Establish a new `(provider, subject) -> user_id` link. The PK
    /// constraint rejects duplicate inserts; callers race only on the
    /// first-login path and the loser sees a `StorageError::Conflict`
    /// (typically resolved by retrying the read path).
    ///
    /// `email` is the IdP-side email AT LINK TIME (audit only). NOT
    /// updated on subsequent logins per Scenario 6.2.
    fn link_external(
        &self,
        user_id: &[u8],
        provider: &str,
        subject: &str,
        email: Option<&str>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;
}
