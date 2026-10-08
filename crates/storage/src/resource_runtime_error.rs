//! Resource-runtime meanings layered over the shared SQL driver classifier.
//!
//! Identity collisions remain conflicts, missing parents remain invisible, and a
//! submitted commit with no acknowledgement remains unknown. Driver failures and
//! persisted-row decoding otherwise follow the common storage classification.

use nebula_storage_port::StorageError;

pub(crate) fn statement_error(error: sqlx::Error) -> StorageError {
    let error = match crate::sql_error::storage_error(error) {
        StorageError::Duplicate { .. } => StorageError::Conflict {
            entity: "resource runtime",
            id: "[opaque]".to_owned(),
            expected: 0,
            actual: 0,
        },
        other => other,
    };
    let outcome = match &error {
        StorageError::Conflict { .. } => "conflict",
        StorageError::Connection(_) => "connection",
        StorageError::Corrupt(_) => "corrupt",
        StorageError::Configuration(_) => "configuration",
        StorageError::InvalidInput(_) => "invalid_input",
        _ => "internal",
    };
    tracing::warn!(
        storage.outcome = outcome,
        "resource runtime statement failed"
    );
    error
}

pub(crate) fn foreign_key_or_statement_error(
    error: sqlx::Error,
    parent: &'static str,
) -> StorageError {
    if crate::sql_error::is_foreign_key_violation(&error) {
        tracing::warn!(
            storage.outcome = "not_found",
            storage.entity = parent,
            "resource runtime parent is missing"
        );
        StorageError::not_found(parent, "[opaque]")
    } else {
        statement_error(error)
    }
}

pub(crate) fn commit_unknown(_error: sqlx::Error) -> StorageError {
    tracing::warn!(
        storage.outcome = "acknowledgement_unknown",
        "resource runtime commit was not acknowledged"
    );
    StorageError::AcknowledgementUnknown {
        operation: "resource runtime mutation",
    }
}

pub(crate) fn corrupt_record() -> StorageError {
    tracing::warn!(
        storage.outcome = "corrupt",
        "resource runtime persisted value is invalid"
    );
    StorageError::Corrupt("resource runtime persisted value is invalid".to_owned())
}

pub(crate) fn generation_exhausted() -> StorageError {
    tracing::warn!(
        storage.outcome = "internal",
        "resource generation exhausted"
    );
    StorageError::Internal("resource generation exhausted".to_owned())
}

pub(crate) fn deadline_overflow() -> StorageError {
    tracing::warn!(
        storage.outcome = "internal",
        "resource lease deadline exceeds its representation"
    );
    StorageError::Internal("resource lease deadline exceeds its representation".to_owned())
}

pub(crate) fn invalid_computed_count() -> StorageError {
    tracing::warn!(
        storage.outcome = "internal",
        "resource runtime computed count is out of range"
    );
    StorageError::Internal("resource runtime computed count is out of range".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_and_record_failures_keep_their_storage_meaning() {
        assert!(matches!(
            statement_error(sqlx::Error::PoolClosed),
            StorageError::Connection(_)
        ));
        assert!(
            matches!(statement_error(sqlx::Error::Decode("secret-value".into())),
            StorageError::Corrupt(message) if !message.contains("secret-value"))
        );
        assert!(matches!(corrupt_record(), StorageError::Corrupt(_)));
        assert!(matches!(generation_exhausted(), StorageError::Internal(_)));
        assert!(matches!(deadline_overflow(), StorageError::Internal(_)));
        assert!(matches!(
            invalid_computed_count(),
            StorageError::Internal(_)
        ));
    }

    #[test]
    fn an_unacknowledged_commit_stays_unknown_regardless_of_driver_cause() {
        assert!(matches!(
            commit_unknown(sqlx::Error::Decode("secret-value".into())),
            StorageError::AcknowledgementUnknown {
                operation: "resource runtime mutation"
            }
        ));
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn resource_identity_conflicts_and_missing_parents_keep_their_domain_meaning() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO parent VALUES (1)")
            .execute(&pool)
            .await
            .unwrap();
        let conflict = sqlx::query("INSERT INTO parent VALUES (1)")
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(matches!(statement_error(conflict), StorageError::Conflict {
            entity: "resource runtime", id, expected: 0, actual: 0
        } if id == "[opaque]"));
        let missing = sqlx::query("INSERT INTO child VALUES (1, 2)")
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(
            matches!(foreign_key_or_statement_error(missing, "shared resource"),
            StorageError::NotFound { entity: "shared resource", id } if id == "[opaque]")
        );
    }
}
