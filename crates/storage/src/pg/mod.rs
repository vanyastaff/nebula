//! PostgreSQL implementations of repository traits.
//!
//! Each module in this directory implements exactly one repo trait from
//! `crate::repos`. All implementations share:
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

use crate::error::StorageError;

mod control_queue;
mod external_identity;
mod idempotency;
mod oauth_state;
mod org;
mod pat;
mod session;
pub(crate) mod user;
mod verification_token;
mod workspace;

pub use control_queue::PgControlQueueRepo;
pub use external_identity::PgExternalIdentityRepo;
pub use idempotency::PgIdempotencyStore;
pub use oauth_state::PgOAuthStateRepo;
pub use org::PgOrgRepo;
pub use pat::PgPatRepo;
pub use session::PgSessionRepo;
pub use user::PgUserRepo;
pub use verification_token::PgVerificationTokenRepo;
pub use workspace::PgWorkspaceRepo;

/// Translate an [`sqlx::Error`] into a [`StorageError`].
///
/// Most errors become [`StorageError::Connection`]. Unique-constraint
/// violations (SQLSTATE `23505`) become [`StorageError::Duplicate`]
/// with the constraint detail preserved.
pub(crate) fn map_db_err(entity: &'static str, err: SqlxError) -> StorageError {
    if let SqlxError::Database(db_err) = &err
        && db_err.code().as_deref() == Some("23505")
    {
        return StorageError::Duplicate {
            entity,
            detail: db_err.message().to_string(),
        };
    }
    StorageError::Connection(err.to_string())
}
