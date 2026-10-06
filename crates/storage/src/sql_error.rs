//! What an `sqlx` failure means for a storage caller — one mapping for both
//! SQL backends.
//!
//! The variant is chosen by *what failed*, not by the call site:
//!
//! - the backend could not be reached, was busy, or cancelled the statement
//!   → [`StorageError::Connection`] (a retry may succeed);
//! - a stored row does not decode into the expected shape
//!   → [`StorageError::Corrupt`] (an operator must look at the data);
//! - a unique constraint rejected a write → [`StorageError::Duplicate`];
//! - the database rejected a statement this code issued (constraint, syntax,
//!   missing expected row) → [`StorageError::Internal`].
//!
//! Messages are value-free: they carry the SQLSTATE code, the constraint
//! name or the column name — never the backend's own message, which can
//! quote the stored or submitted values.

use nebula_storage_port::StorageError;
use sqlx::error::{DatabaseError, ErrorKind};

/// Map an `sqlx` failure to the storage error a caller can act on.
pub(crate) fn storage_error(error: sqlx::Error) -> StorageError {
    match &error {
        sqlx::Error::Database(database) => database_error(database.as_ref()),
        sqlx::Error::ColumnDecode { index, .. } => {
            StorageError::Corrupt(format!("column `{index}` does not decode"))
        },
        sqlx::Error::ColumnNotFound(column) => {
            StorageError::Corrupt(format!("column `{column}` is missing from the row"))
        },
        sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::TypeNotFound { .. } => {
            StorageError::Corrupt("stored row does not match the expected shape".into())
        },
        sqlx::Error::RowNotFound => StorageError::Internal("an expected row is missing".into()),
        sqlx::Error::Configuration(_) => {
            StorageError::Configuration("database driver is misconfigured".into())
        },
        sqlx::Error::Encode(_) => {
            StorageError::InvalidInput("a bound value does not encode for the database".into())
        },
        sqlx::Error::InvalidArgument(_) | sqlx::Error::InvalidSavePointStatement => {
            StorageError::Internal("the driver rejected a statement argument".into())
        },
        // Io, Tls, Protocol, pool exhaustion or closure, a crashed worker: the
        // backend could not be reached — a retry may succeed.
        _ => StorageError::Connection("database backend unavailable".into()),
    }
}

/// [`storage_error`], with a unique violation attributed to `entity` — for
/// stores whose callers branch on which record collided.
pub(crate) fn storage_error_for(entity: &'static str, error: sqlx::Error) -> StorageError {
    match storage_error(error) {
        StorageError::Duplicate { detail, .. } => StorageError::Duplicate { entity, detail },
        other => other,
    }
}

/// Whether a write was rejected because a referenced parent row is missing —
/// for stores that report the missing parent as `NotFound`.
pub(crate) fn is_foreign_key_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| matches!(database.kind(), ErrorKind::ForeignKeyViolation))
}

/// Decode a non-negative counter (version, fencing generation, sequence)
/// stored as a signed SQL integer. A negative value is corrupt data, never a
/// wrapped `u64`.
pub(crate) fn decode_u64(value: i64, column: &'static str) -> Result<u64, StorageError> {
    u64::try_from(value)
        .map_err(|_| StorageError::Corrupt(format!("column `{column}` holds a negative counter")))
}

/// Encode a non-negative counter for a signed 64-bit SQL column. A value
/// past `i64::MAX` is rejected input, never a wrapped negative number.
pub(crate) fn encode_u64(value: u64, column: &'static str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| {
        StorageError::InvalidInput(format!("`{column}` exceeds the signed 64-bit range"))
    })
}

fn database_error(database: &dyn DatabaseError) -> StorageError {
    let code = database.code();
    let code = code.as_deref().unwrap_or("unknown");
    if is_transient(database, code) {
        return StorageError::Connection(format!("database backend unavailable (SQLSTATE {code})"));
    }
    let constraint = database.constraint().unwrap_or("unnamed");
    match database.kind() {
        ErrorKind::UniqueViolation => StorageError::Duplicate {
            entity: "record",
            detail: format!("unique constraint `{constraint}`"),
        },
        ErrorKind::ForeignKeyViolation
        | ErrorKind::NotNullViolation
        | ErrorKind::CheckViolation => StorageError::Internal(format!(
            "database rejected the write: constraint `{constraint}` (SQLSTATE {code})"
        )),
        _ => StorageError::Internal(format!("database rejected the statement (SQLSTATE {code})")),
    }
}

/// Whether a retry can clear the failure. The dialect is decided by the
/// error's type, never by the shape of its code: an all-digit PostgreSQL
/// SQLSTATE such as `42501` would otherwise read as a SQLite code.
fn is_transient(database: &dyn DatabaseError, code: &str) -> bool {
    #[cfg(feature = "sqlite")]
    if database
        .try_downcast_ref::<sqlx::sqlite::SqliteError>()
        .is_some()
    {
        return sqlite_code_is_transient(code);
    }
    #[cfg(not(feature = "sqlite"))]
    let _ = database;
    postgres_code_is_transient(code)
}

/// PostgreSQL connection exceptions (class 08), operator intervention /
/// cancellation (class 57), insufficient resources (class 53), serialization
/// failures and deadlocks (40001, 40P01).
fn postgres_code_is_transient(sqlstate: &str) -> bool {
    ["08", "57", "53"]
        .iter()
        .any(|class| sqlstate.starts_with(class))
        || matches!(sqlstate, "40001" | "40P01")
}

/// SQLite `BUSY` and `LOCKED`: primary codes 5 and 6, including their
/// extended forms (the primary code is the low byte).
#[cfg_attr(
    not(feature = "sqlite"),
    expect(dead_code, reason = "only SQLite errors carry SQLite result codes")
)]
fn sqlite_code_is_transient(code: &str) -> bool {
    code.parse::<u32>()
        .is_ok_and(|extended| matches!(extended & 0xff, 5 | 6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_failures_are_corrupt_and_name_only_the_column() {
        match storage_error(sqlx::Error::ColumnNotFound("status".into())) {
            StorageError::Corrupt(message) => assert!(message.contains("`status`")),
            other => panic!("expected Corrupt, got {other:?}"),
        }
        assert!(matches!(
            storage_error(sqlx::Error::Decode("secret-value".into())),
            StorageError::Corrupt(message) if !message.contains("secret-value")
        ));
    }

    #[test]
    fn unreachable_backends_are_connection_failures() {
        for error in [
            sqlx::Error::PoolTimedOut,
            sqlx::Error::PoolClosed,
            sqlx::Error::WorkerCrashed,
        ] {
            assert!(matches!(storage_error(error), StorageError::Connection(_)));
        }
    }

    #[test]
    fn a_missing_expected_row_is_an_internal_fault() {
        assert!(matches!(
            storage_error(sqlx::Error::RowNotFound),
            StorageError::Internal(_)
        ));
    }

    #[test]
    fn integer_narrowing_never_wraps() {
        assert_eq!(decode_u64(7, "version").ok(), Some(7));
        assert!(matches!(
            decode_u64(-1, "version"),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            encode_u64(u64::MAX, "version"),
            Err(StorageError::InvalidInput(_))
        ));
    }

    #[test]
    fn postgres_transient_codes_are_the_retryable_classes_only() {
        for code in ["08006", "57014", "53300", "40001", "40P01"] {
            assert!(postgres_code_is_transient(code), "{code}");
        }
        // All-digit SQLSTATEs whose low byte is SQLite BUSY/LOCKED stay
        // permanent: insufficient privilege, a NUL byte in text.
        for code in ["23505", "23503", "42601", "42501", "22021", "22022"] {
            assert!(!postgres_code_is_transient(code), "{code}");
        }
    }

    #[test]
    fn sqlite_transient_codes_are_busy_and_locked_only() {
        for code in ["5", "6", "261", "517"] {
            assert!(sqlite_code_is_transient(code), "{code}");
        }
        // Constraint codes, including extended ones that start with "53".
        for code in ["19", "2067", "531", "5386"] {
            assert!(!sqlite_code_is_transient(code), "{code}");
        }
    }

    #[test]
    fn caller_and_driver_faults_are_not_connection_failures() {
        assert!(matches!(
            storage_error(sqlx::Error::InvalidArgument("x".into())),
            StorageError::Internal(_)
        ));
    }
}
