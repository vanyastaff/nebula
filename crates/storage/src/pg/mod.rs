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

mod org;
mod workspace;

pub use org::PgOrgRepo;
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
