//! Account repository traits.

use super::rows::{
    OAuthStateRow, PersonalAccessTokenRow, SessionDraft, SessionRow, UserRow, VerificationTokenRow,
};
use crate::StorageError;

/// User account storage.
#[async_trait::async_trait]
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

    /// Replace an exact active TOTP envelope after authenticating it with an
    /// explicitly configured legacy key. Returns `false` on a benign CAS loss
    /// or an unavailable account. A replacement increments the user version.
    async fn rotate_mfa_secret_envelope(
        &self,
        user_id: &[u8],
        expected_envelope: &[u8],
        replacement_envelope: &[u8],
    ) -> Result<bool, StorageError>;
}

/// Session storage for browser logins.
/// Archived owners hide their sessions from reads and ordinary mutations,
/// including expiry cleanup. Purging the owner removes the retained artifacts.
#[async_trait::async_trait]
pub trait SessionRepo: Send + Sync {
    /// Hash the one-time presented bearer and insert only its digest plus
    /// session metadata. Missing or archived owners return `NotFound`.
    async fn create(
        &self,
        presented_token: &[u8],
        session: &SessionDraft,
    ) -> Result<(), StorageError>;

    /// Fetch by bearer. Returns `None` if missing, revoked, expired, or owner archived.
    async fn get(&self, presented_token: &[u8]) -> Result<Option<SessionRow>, StorageError>;

    /// Touch `last_active_at` to now.
    async fn touch(&self, presented_token: &[u8]) -> Result<(), StorageError>;

    /// Mark the session as revoked.
    async fn revoke(&self, presented_token: &[u8]) -> Result<(), StorageError>;

    /// Delete expired sessions of live owners. Returns the count deleted.
    async fn cleanup_expired(&self) -> Result<u64, StorageError>;
}

/// Personal access token storage.
#[async_trait::async_trait]
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

    /// Revoke only a PAT owned by this principal, including expired tokens.
    /// Returns `true` for an owned row, even if already revoked; foreign-owner
    /// and missing rows both return `false`. Preserve the original revocation time.
    async fn revoke_for_principal(
        &self,
        id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<bool, StorageError>;

    /// List active PATs for a principal.
    async fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Vec<PersonalAccessTokenRow>, StorageError>;
}

/// One-time verification tokens (email verification, password reset,
/// MFA challenges, invitations).
///
/// Tokens are stored by SHA-256 hash of the plaintext value; the
/// plaintext is only available to the caller at mint time and is sent
/// to the user out-of-band (email link, etc.).
#[async_trait::async_trait]
pub trait VerificationTokenRepo: Send + Sync {
    /// Insert a new verification token. Missing or archived owners return `NotFound`.
    async fn create(&self, token: &VerificationTokenRow) -> Result<(), StorageError>;

    /// Atomically mark a token as consumed and return its row. Returns
    /// `None` if the token does not exist, is already consumed, or has
    /// expired, or its owner is archived.
    ///
    /// Prefer [`consume_by_hash_and_kind`](Self::consume_by_hash_and_kind)
    /// for routes that only accept a specific `kind` (e.g. MFA
    /// challenge) — that variant filters on `kind` inside the same SQL
    /// statement, so a token of the wrong kind sent to the wrong
    /// endpoint is rejected as `None` without being burned.
    async fn consume_by_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<VerificationTokenRow>, StorageError>;

    /// Atomically mark a token as consumed **only when both `token_hash`
    /// AND `kind` match** an unconsumed, unexpired row, and return that
    /// row. Returns `None` for any mismatch — including a valid token
    /// presented to the wrong route (where `kind` differs) — so a
    /// password-reset token sent to the MFA-verify endpoint cannot be
    /// destroyed by a blind consume.
    async fn consume_by_hash_and_kind(
        &self,
        token_hash: &[u8],
        kind: &str,
    ) -> Result<Option<VerificationTokenRow>, StorageError>;

    /// Fetch a token by hash without consuming it. Returns `None` if not
    /// found or its owner is archived. Caller checks `expires_at` /
    /// `consumed_at`. Primarily a test helper.
    async fn get_by_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<VerificationTokenRow>, StorageError>;

    /// Delete expired (`expires_at <= now`) tokens of live owners. Returns the
    /// count deleted.
    async fn cleanup_expired(&self) -> Result<u64, StorageError>;

    /// Mark all unconsumed tokens for a user of the given `kind` as
    /// consumed. Used to invalidate in-flight reset / verification
    /// links after a successful action (e.g. password change). Returns
    /// the count revoked. Archived owners are not modified.
    async fn revoke_all_for_user(&self, user_id: &[u8], kind: &str) -> Result<u64, StorageError>;
}

/// Hard ceiling for live, unconsumed Plane-A OAuth states in one deployment.
///
/// The repository, rather than its callers, owns the serialization point that
/// makes the capacity check and insert one atomic decision.
pub const OAUTH_STATE_CAPACITY: u32 = 10_000;

/// Result of attempting to admit one live Plane-A OAuth state.
///
/// The variants deliberately carry no values: neither a state token nor a
/// PKCE verifier can cross diagnostics through this outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "OAuth state admission must be handled before issuing a browser redirect"]
#[non_exhaustive]
pub enum OAuthStateAdmission {
    /// The state row was created within the hard capacity bound.
    Created,
    /// The shared active-state capacity was already exhausted.
    AtCapacity,
    /// Another replica currently owns the non-blocking admission gate.
    Contended,
}

/// Server-side storage for Plane-A OAuth PKCE state.
///
/// Each `start_oauth` mints a row keyed by the random url-safe state
/// string; the matching `complete_oauth` atomically consumes the row
/// to recover the PKCE `code_verifier` and validate the callback.
/// Distinct from the Plane-B credential OAuth surface, which has its
/// own state-pending table in `0006_credentials.sql`.
#[async_trait::async_trait]
pub trait OAuthStateRepo: Send + Sync {
    /// Atomically clean expired rows, enforce [`OAUTH_STATE_CAPACITY`],
    /// and insert a new unconsumed PKCE state row.
    ///
    /// Implementations must fail closed under admission contention rather
    /// than waiting while holding a database connection.
    async fn admit(&self, state: &OAuthStateRow) -> Result<OAuthStateAdmission, StorageError>;

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
    async fn consume_by_state(&self, state: &str) -> Result<Option<OAuthStateRow>, StorageError>;

    /// Atomically mark a PKCE state as consumed **only when both `state`
    /// AND `provider` match** an unconsumed, unexpired row, and return
    /// that row. Returns `None` on any mismatch (including a state value
    /// crossed between providers), so a callback presenting the wrong
    /// provider cannot destroy a valid row.
    async fn consume_by_state_and_provider(
        &self,
        state: &str,
        provider: &str,
    ) -> Result<Option<OAuthStateRow>, StorageError>;

    /// Delete all expired (`expires_at < now`) rows. Returns the count
    /// deleted.
    async fn cleanup_expired(&self) -> Result<u64, StorageError>;

    /// Fetch a state row without consuming it. Returns `None` if not
    /// found. Primarily a test helper — production paths must use
    /// [`consume_by_state`](Self::consume_by_state) so the row cannot
    /// be replayed.
    async fn get_by_state(&self, state: &str) -> Result<Option<OAuthStateRow>, StorageError>;
}

/// Repository for the `external_identities` table (Plane-A OAuth
/// provider ↔ Nebula user linkage). Per ADR-0085 D-8 + REQ-oauth-005
/// / REQ-oauth-006.
///
/// Read path serves the REQ-oauth-006 short-circuit on repeat logins
/// (find_user_by_external returning `Some(user_id)` means the user
/// has logged in via this IdP before. Login finalization owns the atomic
/// account/link/session decision; an email match alone never authorizes a link.
#[async_trait::async_trait]
pub trait ExternalIdentityRepo: Send + Sync {
    /// Resolve `(provider, subject)` to a Nebula `user_id`. Returns
    /// `None` when there is no existing link — the caller then falls
    /// through to login finalization, which distinguishes a new account from
    /// an email collision requiring explicit account linking.
    async fn find_user_by_external(
        &self,
        provider: &str,
        subject: &str,
    ) -> Result<Option<Vec<u8>>, StorageError>;

    /// Establish a new `(provider, subject) -> user_id` link. The PK
    /// constraint rejects duplicate inserts; callers race only on the
    /// first-login path and the loser sees a `StorageError::Duplicate`
    /// (typically resolved by retrying the read path). Per CodeRabbit
    /// wave-1 H.4: the Postgres `map_db_err` helper emits
    /// `Duplicate` (not `Conflict`) on SQLSTATE 23505.
    ///
    /// `email` is the IdP-side email AT LINK TIME (audit only). NOT
    /// updated on subsequent logins per Scenario 6.2.
    async fn link_external(
        &self,
        user_id: &[u8],
        provider: &str,
        subject: &str,
        email: Option<&str>,
    ) -> Result<(), StorageError>;
}

#[cfg(test)]
mod oauth_state_admission_contract_tests {
    use super::{OAUTH_STATE_CAPACITY, OAuthStateAdmission};

    #[test]
    fn oauth_state_capacity_is_the_platform_hard_limit() {
        assert_eq!(OAUTH_STATE_CAPACITY, 10_000);
    }

    #[test]
    fn oauth_state_admission_outcomes_have_no_secret_bearing_payload() {
        let outcomes = [
            OAuthStateAdmission::Created,
            OAuthStateAdmission::AtCapacity,
            OAuthStateAdmission::Contended,
        ];

        assert_eq!(
            outcomes.map(|outcome| format!("{outcome:?}")),
            ["Created", "AtCapacity", "Contended"]
        );
    }
}
