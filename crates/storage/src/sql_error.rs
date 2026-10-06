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
        _ => StorageError::Connection("database backend unavailable".into()),
    }
}

/// Decode a non-negative counter (version, fencing generation, sequence)
/// stored as a signed SQL integer. A negative value is corrupt data, never a
/// wrapped `u64`.
pub(crate) fn decode_u64(value: i64, column: &'static str) -> Result<u64, StorageError> {
    u64::try_from(value)
        .map_err(|_| StorageError::Corrupt(format!("column `{column}` holds a negative counter")))
}

fn database_error(database: &dyn DatabaseError) -> StorageError {
    let code = database.code();
    let code = code.as_deref().unwrap_or("unknown");
    if is_transient(code) {
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

/// Failures a retry can clear: PostgreSQL connection exceptions (class 08),
/// operator intervention / cancellation (class 57), insufficient resources
/// (class 53), serialization failures and deadlocks (40001, 40P01); SQLite
/// `BUSY` and `LOCKED` (primary codes 5 and 6, including extended forms).
fn is_transient(code: &str) -> bool {
    if ["08", "57", "53"]
        .iter()
        .any(|class| code.starts_with(class))
        || matches!(code, "40001" | "40P01")
    {
        return true;
    }
    code.parse::<u32>()
        .is_ok_and(|sqlite| matches!(sqlite & 0xff, 5 | 6))
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
    fn transient_codes_cover_both_dialects() {
        for code in [
            "08006", "57014", "53300", "40001", "40P01", "5", "6", "261", "517",
        ] {
            assert!(is_transient(code), "{code}");
        }
        for code in ["23505", "23503", "42601", "19", "2067"] {
            assert!(!is_transient(code), "{code}");
        }
    }
}
