//! PostgreSQL implementations of the Plane-A account repositories.
//!
//! Each module in this directory implements exactly one repo trait from
//! `crate::repos`, plus the identity-secret rotation migrator. All
//! implementations share:
//!
//! - a `sqlx::Pool<Postgres>` for connection management
//! - the `map_db_err` helper for translating `sqlx::Error` into `StorageError`
//! - SQLSTATE `23505` (unique violation) → `StorageError::Duplicate`
//!
//! # Testing
//!
//! Tests are gated behind `cfg(all(test, feature = "postgres"))` and
//! are skipped when `DATABASE_URL` is not set in the environment.

use sqlx::Error as SqlxError;

use crate::StorageError;

mod external_identity;
mod idempotency;
mod identity_secret;
mod mfa_enrollment;
mod oauth_login;
mod oauth_state;
mod pat;
mod session;
pub(crate) mod user;
mod verification_token;

pub use external_identity::PgExternalIdentityRepo;
pub use idempotency::PgIdempotencyStore;
pub use identity_secret::{
    IdentitySecretMigrationError, IdentitySecretMigrationReport, IdentitySecretRejectionReason,
    PgIdentitySecretMigrator,
};
pub use mfa_enrollment::PgMfaEnrollmentRepo;
pub use oauth_login::PgOAuthLoginFinalizer;
pub use oauth_state::PgOAuthStateRepo;
pub use pat::PgPatRepo;
pub use session::PgSessionRepo;
pub use user::PgUserRepo;
pub use verification_token::PgVerificationTokenRepo;

/// Translate an [`sqlx::Error`] into a [`StorageError`].
///
/// The shared classification ([`crate::sql_error::storage_error`]), with a
/// unique violation attributed to `entity` and its constraint name — never
/// the backend message, which can quote the colliding value.
pub(crate) fn map_db_err(entity: &'static str, err: SqlxError) -> StorageError {
    if let SqlxError::Database(db_err) = &err
        && db_err.kind() == sqlx::error::ErrorKind::UniqueViolation
    {
        return StorageError::Duplicate {
            entity,
            detail: format!(
                "unique constraint `{}`",
                db_err.constraint().unwrap_or("unnamed")
            ),
        };
    }
    crate::sql_error::storage_error(err)
}
