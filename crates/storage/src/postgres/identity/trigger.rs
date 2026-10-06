//! `port_triggers`: workspace-scoped workflow triggers.

use nebula_storage_port::dto::TriggerRow;
use nebula_storage_port::store::TriggerStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use sqlx::types::Json;

use super::{
    cas_disambiguate_scoped, encode_version, json, optional, required, soft_delete_scoped, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// Postgres-backed `triggers` store.
#[derive(Clone, Debug)]
pub struct PgTriggerStore {
    pool: PgPool,
}

impl PgTriggerStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn decode_trigger(row: &PgRow) -> Result<TriggerRow, StorageError> {
    Ok(TriggerRow {
        id: required(row, "id")?,
        workspace_id: required(row, "workspace_id")?,
        workflow_id: required(row, "workflow_id")?,
        slug: required(row, "slug")?,
        display_name: required(row, "display_name")?,
        kind: required(row, "kind")?,
        config: json(row, "config")?,
        state: required(row, "state")?,
        run_as: optional(row, "run_as")?,
        webhook_path: optional(row, "webhook_path")?,
        created_at: required(row, "created_at")?,
        created_by: required(row, "created_by")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?,
    })
}

#[async_trait::async_trait]
impl TriggerStore for PgTriggerStore {
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_triggers (id, workspace_id, org_id, workflow_id, \
             slug, display_name, kind, config, state, run_as, webhook_path, \
             created_at, created_by, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
        .bind(&row.state)
        .bind(&row.run_as)
        .bind(&row.webhook_path)
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(encode_version(row.version)?)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("trigger", error))?;
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<TriggerRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_triggers \
             WHERE workspace_id = $1 AND org_id = $2 AND id = $3 AND deleted_at IS NULL",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_trigger)
        .transpose()
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<TriggerRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_triggers \
             WHERE workspace_id = $1 AND org_id = $2 AND deleted_at IS NULL ORDER BY id",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .iter()
        .map(decode_trigger)
        .collect()
    }

    async fn update(
        &self,
        scope: &Scope,
        row: TriggerRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_triggers SET workflow_id = $1, slug = $2, \
             display_name = $3, kind = $4, config = $5, state = $6, \
             run_as = $7, webhook_path = $8, version = $9 \
             WHERE workspace_id = $10 AND org_id = $11 AND id = $12 \
             AND deleted_at IS NULL AND version = $13",
        )
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
        .bind(&row.state)
        .bind(&row.run_as)
        .bind(&row.webhook_path)
        .bind(encode_version(row.version)?)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.id)
        .bind(encode_version(expected_version)?)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("trigger", error))?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        cas_disambiguate_scoped(
            &self.pool,
            "port_triggers",
            "trigger",
            scope,
            &row.id,
            expected_version,
        )
        .await
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        soft_delete_scoped(&self.pool, "port_triggers", "trigger", scope, id).await
    }
}
