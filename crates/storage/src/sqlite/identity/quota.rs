//! `port_quotas`: per-org limits and the concurrent-execution counter.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::QuotaRow;
use nebula_storage_port::store::QuotaStore;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;

use super::{int32, optional, required};
use crate::sql_error::{decode_i32, decode_u64, storage_error};

/// Why the guarded counter `UPDATE` matched no row, given the counter read
/// afterwards: no quota row ⇒ `NotFound`; past `i32::MAX` ⇒ `InvalidInput`;
/// below zero ⇒ `Conflict`.
fn adjustment_refused(current: Option<i64>, org_id: &str, delta: i32) -> StorageError {
    let Some(actual) = current else {
        return StorageError::not_found("quota", org_id);
    };
    if actual.saturating_add(i64::from(delta)) > i64::from(i32::MAX) {
        return StorageError::InvalidInput("concurrent execution adjustment overflows".into());
    }
    match decode_u64(actual, "concurrent_executions") {
        Ok(actual) => StorageError::Conflict {
            entity: "quota",
            id: org_id.to_owned(),
            expected: 0,
            actual,
        },
        Err(corrupt) => corrupt,
    }
}

/// SQLite-backed `org_quotas` + `org_quota_usage` store.
#[derive(Clone, Debug)]
pub struct SqliteQuotaStore {
    pool: SqlitePool,
}

impl SqliteQuotaStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_quota(row: &SqliteRow) -> Result<QuotaRow, StorageError> {
    Ok(QuotaRow {
        org_id: required(row, "org_id")?,
        plan: required(row, "plan")?,
        concurrent_executions_limit: int32(row, "concurrent_executions_limit")?,
        executions_per_month_limit: optional(row, "executions_per_month_limit")?,
        active_workflows_limit: optional::<i64>(row, "active_workflows_limit")?
            .map(|limit| decode_i32(limit, "active_workflows_limit"))
            .transpose()?,
        concurrent_executions: int32(row, "concurrent_executions")?,
        executions_this_month: required(row, "executions_this_month")?,
        month_reset_at: required(row, "month_reset_at")?,
        updated_at: required(row, "updated_at")?,
    })
}

#[async_trait::async_trait]
impl QuotaStore for SqliteQuotaStore {
    async fn get(&self, org_id: &str) -> Result<Option<QuotaRow>, StorageError> {
        sqlx::query("SELECT * FROM port_quotas WHERE org_id = ?")
            .bind(org_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_quota)
            .transpose()
    }

    async fn upsert(&self, row: QuotaRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_quotas (org_id, plan, concurrent_executions_limit, \
             executions_per_month_limit, active_workflows_limit, \
             concurrent_executions, executions_this_month, month_reset_at, \
             updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (org_id) DO UPDATE SET plan = excluded.plan, \
             concurrent_executions_limit = excluded.concurrent_executions_limit, \
             executions_per_month_limit = excluded.executions_per_month_limit, \
             active_workflows_limit = excluded.active_workflows_limit, \
             concurrent_executions = excluded.concurrent_executions, \
             executions_this_month = excluded.executions_this_month, \
             month_reset_at = excluded.month_reset_at, \
             updated_at = excluded.updated_at",
        )
        .bind(&row.org_id)
        .bind(&row.plan)
        .bind(i64::from(row.concurrent_executions_limit))
        .bind(row.executions_per_month_limit)
        .bind(row.active_workflows_limit.map(i64::from))
        .bind(i64::from(row.concurrent_executions))
        .bind(row.executions_this_month)
        .bind(&row.month_reset_at)
        .bind(&row.updated_at)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn adjust_concurrent(&self, org_id: &str, delta: i32) -> Result<i32, StorageError> {
        // Both bounds are enforced in the WHERE: an adjustment that would go
        // negative or past `i32::MAX` affects zero rows and is rejected below,
        // so the stored counter always decodes.
        let updated = sqlx::query_scalar::<_, i64>(
            "UPDATE port_quotas \
             SET concurrent_executions = concurrent_executions + ?1 \
             WHERE org_id = ?2 AND concurrent_executions + ?1 BETWEEN 0 AND ?3 \
             RETURNING concurrent_executions",
        )
        .bind(i64::from(delta))
        .bind(org_id)
        .bind(i64::from(i32::MAX))
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        if let Some(current) = updated {
            return decode_i32(current, "concurrent_executions");
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT concurrent_executions FROM port_quotas WHERE org_id = ?",
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        Err(adjustment_refused(current, org_id, delta))
    }
}
