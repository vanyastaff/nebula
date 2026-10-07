//! Postgres identity-zoo stores over the port-scoped schema — one file per
//! aggregate, sharing the column decoders, CAS helpers and advisory locks
//! below.
//!
//! Each aggregate is a table in the ordered PostgreSQL migrations. Every tenant- or
//! parent-scoped query carries its scope predicate (`WHERE org_id = $1`,
//! `WHERE workspace_id = $1 AND org_id = $2`, …) and active-row reads add
//! `AND deleted_at IS NULL`, so a cross-scope `get` yields `Ok(None)` and
//! a cross-scope `update` / `soft_delete` is `NotFound` — an id outside
//! the caller's scope is indistinguishable from one that does not exist
//! (no existence oracle, spec §6.1), exactly as the in-memory and SQLite
//! backends behave.
//!
//! First-writer-wins uniqueness (email / slug among *active* rows) is a
//! partial unique index `WHERE deleted_at IS NULL`, so a soft-deleted row
//! frees its key. Optimistic CAS is a single conditional `UPDATE … WHERE
//! version = $N` followed by a disambiguating read (gone ⇒ `NotFound`,
//! moved ⇒ `Conflict`). JSON columns are `JSONB` mapped through
//! [`sqlx::types::Json`]; binary columns are `BYTEA`. A stored value that
//! does not decode is [`StorageError::Corrupt`].

mod membership;
mod org;
mod resource;
mod tenant_provisioning;
mod trigger;
mod workspace;

pub use membership::PgMembershipStore;
pub use org::PgOrgStore;
pub use resource::PgResourceStore;
pub use tenant_provisioning::PgTenantProvisioningStore;
pub use trigger::PgTriggerStore;
pub use workspace::PgWorkspaceStore;

use chrono::{DateTime, SecondsFormat, Utc};
use nebula_storage_port::{MicrosInstant, Scope, StorageError};
use serde::de::DeserializeOwned;
use sqlx::postgres::PgRow;
use sqlx::types::Json;
use sqlx::{PgConnection, PgPool, Row};

use crate::sql_error::{decode_u64, encode_u64, storage_error};

// ── column decoders ──────────────────────────────────────────────────────

/// Decode a NOT NULL column; SQL NULL or a type mismatch is
/// [`StorageError::Corrupt`].
fn required<'r, T>(row: &'r PgRow, column: &'static str) -> Result<T, StorageError>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(column).map_err(storage_error)
}

/// Decode a nullable column; a value that does not decode is an error, never
/// a silent `None`.
fn optional<'r, T>(row: &'r PgRow, column: &'static str) -> Result<Option<T>, StorageError>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get::<Option<T>, _>(column).map_err(storage_error)
}

/// The row's optimistic-concurrency `version`.
fn version(row: &PgRow) -> Result<u64, StorageError> {
    decode_u64(required(row, "version")?, "version")
}

/// A NOT NULL JSONB column.
fn json<T: DeserializeOwned>(row: &PgRow, column: &'static str) -> Result<T, StorageError> {
    required::<Json<T>>(row, column).map(|Json(value)| value)
}

/// A nullable JSONB column.
fn optional_json<T: DeserializeOwned>(
    row: &PgRow,
    column: &'static str,
) -> Result<Option<T>, StorageError> {
    optional::<Json<T>>(row, column).map(|value| value.map(|Json(value)| value))
}

fn encode_version(version: u64) -> Result<i64, StorageError> {
    encode_u64(version, "version")
}

/// A port instant carried as RFC 3339 text (`entity`'s `column`), at
/// storage precision.
fn encode_text_instant(
    entity: &'static str,
    value: &str,
    column: &'static str,
) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|instant| MicrosInstant::floor(instant.with_timezone(&Utc)).to_datetime())
        .map_err(|_| StorageError::InvalidInput(format!("{entity} `{column}` is not RFC 3339")))
}

/// A stored instant as the port's RFC 3339 text.
fn decode_text_instant(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

// ── advisory locks ───────────────────────────────────────────────────────

/// Hold a transaction-scoped advisory lock on `key`.
async fn advisory_xact_lock(connection: &mut PgConnection, key: &str) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(key)
        .execute(connection)
        .await
        .map_err(storage_error)?;
    Ok(())
}

// ── live parents ─────────────────────────────────────────────────────────

/// Share-lock `scope`'s workspace while it and its org are live, so a
/// concurrent archive serializes with a write beneath it (the foreign key
/// proves existence only); missing or archived is `NotFound`.
pub(crate) async fn lock_live_workspace(
    connection: &mut PgConnection,
    scope: &Scope,
) -> Result<(), StorageError> {
    let workspace = sqlx::query(
        "SELECT w.id FROM workspaces w JOIN orgs o ON o.id = w.org_id \
         WHERE w.org_id = $1 AND w.id = $2 \
           AND w.deleted_at IS NULL AND o.deleted_at IS NULL \
         FOR SHARE OF w",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    workspace
        .map(|_| ())
        .ok_or_else(|| StorageError::not_found("workspace", scope.workspace_id.clone()))
}

// ── CAS and soft delete ──────────────────────────────────────────────────

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
    pool: &PgPool,
    table: &str,
    entity: &'static str,
    id: &str,
    expected_version: u64,
) -> Result<(), StorageError> {
    // `table` is a fixed internal literal (never user input), so the
    // format here cannot be an injection vector.
    let sql = format!("SELECT version FROM {table} WHERE id = $1");
    let current = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(storage_error)?;
    Err(cas_failure(current, entity, id, expected_version))
}

/// Explain a zero-row CAS `UPDATE` on a workspace-scoped soft-deletable
/// table: an archived row is `NotFound`, never a spurious `Conflict`.
async fn cas_disambiguate_scoped(
    pool: &PgPool,
    table: &str,
    entity: &'static str,
    scope: &Scope,
    id: &str,
    expected_version: u64,
) -> Result<(), StorageError> {
    let sql = format!(
        "SELECT version FROM {table} \
         WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL"
    );
    let current = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(storage_error)?;
    Err(cas_failure(current, entity, id, expected_version))
}

/// Soft-delete a single-PK `id` row of a `TIMESTAMPTZ` table (active rows
/// only), stamped by the database clock; zero rows ⇒ `NotFound`.
async fn soft_delete_by_id(
    pool: &PgPool,
    table: &str,
    entity: &'static str,
    id: &str,
) -> Result<(), StorageError> {
    let sql = format!("UPDATE {table} SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL");
    let res = sqlx::query(sqlx::AssertSqlSafe(sql))
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

/// Soft-delete a workspace-scoped `id` row of a `TIMESTAMPTZ` table (active
/// rows only), stamped by the database clock; zero rows ⇒ `NotFound`.
async fn soft_delete_scoped(
    pool: &PgPool,
    table: &str,
    entity: &'static str,
    scope: &Scope,
    id: &str,
) -> Result<(), StorageError> {
    let sql = format!(
        "UPDATE {table} SET deleted_at = now() \
         WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL"
    );
    let res = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
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
