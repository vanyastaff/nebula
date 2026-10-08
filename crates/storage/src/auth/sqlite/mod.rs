//! Account persistence on the application's admitted SQLite deployment pool.
//! Instants cross the database boundary as integer Unix microseconds.

mod account_lifecycle;
mod identity_secret;
mod initial_owner;
mod mfa_enrollment;
mod oauth_login;
mod oauth_state;
mod pat;
mod session;
mod user;
mod verification_token;

pub use account_lifecycle::SqliteAccountLifecycle;
pub use identity_secret::admit_identity_secrets;
pub use mfa_enrollment::SqliteMfaEnrollmentRepo;
pub use oauth_login::SqliteOAuthLoginFinalizer;
pub use oauth_state::SqliteOAuthStateRepo;
pub use pat::SqlitePatRepo;
pub use session::SqliteSessionRepo;
pub use user::SqliteUserRepo;
pub use verification_token::SqliteVerificationTokenRepo;

use chrono::{DateTime, Utc};
use sqlx::{Row, sqlite::SqliteRow};

use crate::{StorageError, sql_error::storage_error};

const NOW: &str = "CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER)";

fn decode_instant(value: i64) -> Result<DateTime<Utc>, StorageError> {
    DateTime::from_timestamp_micros(value)
        .ok_or_else(|| StorageError::Corrupt("identity timestamp is out of range".into()))
}

fn instant(row: &SqliteRow, column: &'static str) -> Result<DateTime<Utc>, StorageError> {
    decode_instant(row.try_get(column).map_err(storage_error)?)
}

fn optional_instant(
    row: &SqliteRow,
    column: &'static str,
) -> Result<Option<DateTime<Utc>>, StorageError> {
    row.try_get::<Option<i64>, _>(column)
        .map_err(storage_error)?
        .map(decode_instant)
        .transpose()
}

fn micros(value: Option<DateTime<Utc>>) -> Option<i64> {
    value.map(|value| value.timestamp_micros())
}
