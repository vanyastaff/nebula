//! SQLite identity-zoo stores over the port-scoped schema — one file per
//! aggregate, sharing the column decoders and CAS helpers below.
//!
//! Each aggregate is a table in the ordered SQLite migrations. Every tenant- or
//! parent-scoped query carries its scope predicate (`WHERE org_id = ?`,
//! `WHERE workspace_id = ? AND org_id = ?`, …) and active-row reads add
//! `AND deleted_at IS NULL`, so a cross-scope `get` yields `Ok(None)` and
//! a cross-scope `update` / `soft_delete` is `NotFound` — an id outside
//! the caller's scope is indistinguishable from one that does not exist
//! (no existence oracle, spec §6.1), exactly as the in-memory backend
//! behaves.
//!
//! First-writer-wins uniqueness (email / slug among *active* rows) is a
//! partial unique index `WHERE deleted_at IS NULL`, so a soft-deleted row
//! frees its key. Optimistic CAS is a single conditional `UPDATE … WHERE
//! version = ?` followed by a disambiguating read (zero rows ⇒ the row is
//! gone → `NotFound`, or the version moved → `Conflict`). JSON columns are
//! opaque TEXT round-tripped through `serde_json`; binary columns are
//! `BLOB`. A stored value that does not decode is [`StorageError::Corrupt`].

mod membership;
mod org;
mod resource;
mod tenant_provisioning;
mod trigger;
mod workspace;

pub use membership::SqliteMembershipStore;
pub use org::SqliteOrgStore;
pub use resource::SqliteResourceStore;
pub use tenant_provisioning::SqliteTenantProvisioningStore;
pub use trigger::SqliteTriggerStore;
pub use workspace::SqliteWorkspaceStore;

use chrono::{DateTime, Utc};
use nebula_storage_port::{Scope, StorageError};
use serde::de::DeserializeOwned;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::sql_error::{decode_u64, encode_u64, storage_error};

// ── column decoders ──────────────────────────────────────────────────────

/// Decode a NOT NULL column; SQL NULL is [`StorageError::Corrupt`].
///
/// sqlx's SQLite backend maps NULL to the zero value for scalar types (`""`
/// for `String`, `0` for `i64`, …), so a bare `try_get::<T>` silently accepts
/// NULL. Decoding as `Option<T>` yields `None` for NULL regardless of `T`.
fn required<'r, T>(row: &'r SqliteRow, column: &'static str) -> Result<T, StorageError>
where
    T: sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    optional(row, column)?
        .ok_or_else(|| StorageError::Corrupt(format!("NOT NULL column `{column}` is NULL")))
}

/// Decode a nullable column; a value that does not decode is an error, never
/// a silent `None`.
fn optional<'r, T>(row: &'r SqliteRow, column: &'static str) -> Result<Option<T>, StorageError>
where
    T: sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    row.try_get::<Option<T>, _>(column).map_err(storage_error)
}

/// The row's optimistic-concurrency `version`.
fn version(row: &SqliteRow) -> Result<u64, StorageError> {
    decode_u64(required(row, "version")?, "version")
}

/// A boolean stored as an INTEGER `0` / `1`.
fn flag(row: &SqliteRow, column: &'static str) -> Result<bool, StorageError> {
    Ok(required::<i64>(row, column)? != 0)
}

/// A NOT NULL JSON TEXT column.
fn json<T: DeserializeOwned>(row: &SqliteRow, column: &'static str) -> Result<T, StorageError> {
    parse_json(&required::<String>(row, column)?, column)
}

/// A nullable JSON TEXT column; NULL and the empty string are `None`.
fn optional_json<T: DeserializeOwned>(
    row: &SqliteRow,
    column: &'static str,
) -> Result<Option<T>, StorageError> {
    match optional::<String>(row, column)? {
        Some(text) if !text.is_empty() => parse_json(&text, column).map(Some),
        _ => Ok(None),
    }
}

fn parse_json<T: DeserializeOwned>(text: &str, column: &'static str) -> Result<T, StorageError> {
    serde_json::from_str(text)
        .map_err(|_| StorageError::Corrupt(format!("column `{column}` is not the expected JSON")))
}

/// A NOT NULL instant stored as INTEGER microseconds since the Unix epoch.
fn instant(row: &SqliteRow, column: &'static str) -> Result<DateTime<Utc>, StorageError> {
    decode_instant(required(row, column)?, column)
}

/// A nullable instant stored as INTEGER microseconds since the Unix epoch.
fn optional_instant(
    row: &SqliteRow,
    column: &'static str,
) -> Result<Option<DateTime<Utc>>, StorageError> {
    optional(row, column)?
        .map(|micros| decode_instant(micros, column))
        .transpose()
}

fn decode_instant(micros: i64, column: &'static str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| StorageError::Corrupt(format!("column `{column}` is not a valid instant")))
}

// ── column encoders ──────────────────────────────────────────────────────

fn json_text(value: &serde_json::Value) -> String {
    value.to_string()
}

fn encode_version(version: u64) -> Result<i64, StorageError> {
    encode_u64(version, "version")
}

/// An instant as INTEGER microseconds since the Unix epoch (sub-microsecond
/// precision is truncated, as in PostgreSQL `TIMESTAMPTZ`).
fn encode_instant(instant: DateTime<Utc>) -> i64 {
    instant.timestamp_micros()
}

/// The current instant as INTEGER microseconds since the Unix epoch.
fn now_micros() -> i64 {
    encode_instant(Utc::now())
}

// ── CAS and soft delete ──────────────────────────────────────────────────

/// Current time as an RFC 3339 string — the soft-delete stamp of the
/// aggregates that still store instants as text (resources).
fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// The error for a zero-row CAS `UPDATE`, given the row's current version:
/// the row is gone (or soft-deleted) ⇒ `NotFound`; the version moved ⇒
/// `Conflict { actual }`.
fn cas_failure(
    current: Option<i64>,
    entity: &'static str,
    id: impl Into<String>,
    expected: u64,
) -> StorageError {
    let id = id.into();
    match current.map(|actual| decode_u64(actual, "version")) {
        Some(Ok(actual)) => StorageError::Conflict {
            entity,
            id,
            expected,
            actual,
        },
        Some(Err(corrupt)) => corrupt,
        None => StorageError::not_found(entity, id),
    }
}

/// Explain a zero-row CAS `UPDATE` on a single-PK `id` table.
async fn cas_disambiguate(
    pool: &SqlitePool,
    table: &str,
    entity: &'static str,
    id: &str,
    expected_version: u64,
) -> Result<(), StorageError> {
    // `table` is a fixed internal literal (never user input), so the
    // format here cannot be an injection vector.
    let sql = format!("SELECT version FROM {table} WHERE id = ?");
    let current = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(storage_error)?;
    Err(cas_failure(current, entity, id, expected_version))
}

/// Explain a zero-row CAS `UPDATE` on a workspace-scoped table.
async fn cas_disambiguate_scoped(
    pool: &SqlitePool,
    table: &str,
    entity: &'static str,
    scope: &Scope,
    id: &str,
    expected_version: u64,
) -> Result<(), StorageError> {
    let sql =
        format!("SELECT version FROM {table} WHERE workspace_id = ? AND org_id = ? AND id = ?");
    let current = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(storage_error)?;
    Err(cas_failure(current, entity, id, expected_version))
}

/// Soft-delete a single-PK `id` row of a microsecond-instant table (active
/// rows only); zero rows ⇒ `NotFound`.
async fn soft_delete_by_id(
    pool: &SqlitePool,
    table: &str,
    entity: &'static str,
    id: &str,
) -> Result<(), StorageError> {
    let sql = format!("UPDATE {table} SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL");
    let res = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(now_micros())
        .bind(id)
        .execute(pool)
        .await
        .map_err(storage_error)?;
    if res.rows_affected() > 0 {
        Ok(())
    } else {
        Err(StorageError::not_found(entity, id))
    }
}

/// Soft-delete a workspace-scoped `id` row (active rows only); zero rows ⇒
/// `NotFound`.
async fn soft_delete_scoped(
    pool: &SqlitePool,
    table: &str,
    entity: &'static str,
    scope: &Scope,
    id: &str,
) -> Result<(), StorageError> {
    let sql = format!(
        "UPDATE {table} SET deleted_at = ? \
         WHERE workspace_id = ? AND org_id = ? AND id = ? AND deleted_at IS NULL"
    );
    let res = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(now_rfc3339())
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .execute(pool)
        .await
        .map_err(storage_error)?;
    if res.rows_affected() > 0 {
        Ok(())
    } else {
        Err(StorageError::not_found(entity, id))
    }
}

#[cfg(test)]
mod decoder_tests;
