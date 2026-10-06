//! `port_audit_log`: append-only; reads are newest-first.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::AuditLogRow;
use nebula_storage_port::store::AuditStore;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;

use super::{json_text, optional, optional_json, required};
use crate::sql_error::storage_error;

/// SQLite-backed `audit_log` store.
#[derive(Clone, Debug)]
pub struct SqliteAuditStore {
    pool: SqlitePool,
}

impl SqliteAuditStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_audit(row: &SqliteRow) -> Result<AuditLogRow, StorageError> {
    Ok(AuditLogRow {
        id: required(row, "id")?,
        org_id: required(row, "org_id")?,
        workspace_id: optional(row, "workspace_id")?,
        actor_kind: required(row, "actor_kind")?,
        actor_id: optional(row, "actor_id")?,
        action: required(row, "action")?,
        target_kind: optional(row, "target_kind")?,
        target_id: optional(row, "target_id")?,
        details: optional_json(row, "details")?,
        ip_address: optional(row, "ip_address")?,
        user_agent: optional(row, "user_agent")?,
        emitted_at: required(row, "emitted_at")?,
    })
}

#[async_trait::async_trait]
impl AuditStore for SqliteAuditStore {
    async fn append(&self, row: AuditLogRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_audit_log (id, org_id, workspace_id, actor_kind, \
             actor_id, action, target_kind, target_id, details, ip_address, \
             user_agent, emitted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&row.org_id)
        .bind(&row.workspace_id)
        .bind(&row.actor_kind)
        .bind(&row.actor_id)
        .bind(&row.action)
        .bind(&row.target_kind)
        .bind(&row.target_id)
        .bind(row.details.as_ref().map(json_text))
        .bind(&row.ip_address)
        .bind(&row.user_agent)
        .bind(&row.emitted_at)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn list_for_org(
        &self,
        org_id: &str,
        limit: u32,
    ) -> Result<Vec<AuditLogRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_audit_log WHERE org_id = ? \
             ORDER BY emitted_at DESC, id DESC LIMIT ?",
        )
        .bind(org_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .iter()
        .map(decode_audit)
        .collect()
    }
}
