//! Atomic account transitions that span users and one-time verification tokens.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::StorageError;

/// Password signup prepared by the authentication owner before entering storage.
///
/// Password hashing and token minting happen outside the transaction. Storage
/// creates an unverified account and its verification token together; callers
/// send the plaintext token only after this operation commits.
pub struct PasswordRegistration<'a> {
    /// New user identifier in the account repository's raw byte representation.
    pub user_id: &'a [u8],
    /// Normalized email already admitted by authentication policy.
    pub email: &'a str,
    /// Validated display name.
    pub display_name: &'a str,
    /// Encoded password hash; never a plaintext password.
    pub password_hash: &'a str,
    /// SHA-256 digest of the one-time email verification token.
    pub verification_hash: &'a [u8; 32],
    /// Account and verification-token creation time.
    pub created_at: DateTime<Utc>,
    /// Verification-token deadline chosen by authentication policy.
    pub verification_expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for PasswordRegistration<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PasswordRegistration")
            .field("created_at", &self.created_at)
            .field("verification_expires_at", &self.verification_expires_at)
            .finish_non_exhaustive()
    }
}

/// Result of a one-time account-token transition. Rejections change no rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum AccountTokenOutcome {
    /// Token consumption and the requested account change committed together.
    Applied,
    /// The token is missing, consumed, expired or intended for another operation.
    InvalidToken,
    /// The token names an account that is missing or archived.
    UserUnavailable,
}

/// Owns short transactions across the account and its verification tokens.
///
/// Implementations perform no password hashing, network I/O or email delivery.
/// Semantic rejection or failure before commit rolls back all writes together.
/// A commit transport failure can have an unknown outcome: the complete change
/// may already be durable. Backend faults use value-free [`StorageError`]s;
/// callers must not infer rollback from an error returned during commit.
///
/// # Cancellation
/// Cancellation before commit abandons the transaction. Cancellation while
/// commit is in flight may leave the whole operation committed; it never proves
/// rollback. These operations guarantee atomic writes, not receipt of commit.
pub trait AccountLifecycle: Send + Sync {
    /// Create an unverified account and its email verification token atomically.
    ///
    /// # Errors
    /// An active email collision returns `Duplicate { entity: "user", .. }`.
    /// Constraint and pre-commit database failures abort both inserts. Commit
    /// failures have the unknown-outcome semantics described on this trait.
    fn register_password_user(
        &self,
        registration: &PasswordRegistration<'_>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Consume a live email-verification token and mark its live account verified.
    ///
    /// # Errors
    /// Returns a storage failure if the atomic transition cannot complete.
    fn verify_email(
        &self,
        token_hash: &[u8; 32],
    ) -> impl Future<Output = Result<AccountTokenOutcome, StorageError>> + Send;

    /// Consume a live password-reset token, install the prepared hash, clear
    /// login lockout and consume every unconsumed sibling reset token.
    /// MFA and email-verification state are preserved.
    ///
    /// # Errors
    /// Returns a storage failure if the atomic transition cannot complete.
    fn reset_password(
        &self,
        token_hash: &[u8; 32],
        password_hash: &str,
    ) -> impl Future<Output = Result<AccountTokenOutcome, StorageError>> + Send;
}
