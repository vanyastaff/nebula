//! Postgres `WorkflowStore` + `WorkflowVersionStore` (spec-16 split) over
//! the port-scoped schema.
//!
//! The workflow row (id / slug / soft-delete / CAS version) and its
//! versions (each carrying the opaque definition payload) are separate
//! tables. Every query carries `WHERE workspace_id = $ AND org_id = $`, so
//! a cross-tenant `get` yields `Ok(None)` and a cross-tenant `update` /
//! `soft_delete` is a `NotFound` — an id outside the caller's scope is
//! indistinguishable from one that does not exist (no existence oracle),
//! exactly as the in-memory and SQLite backends behave.
//!
//! `get_published` returns the **highest-numbered** published version
//! (`ORDER BY number DESC LIMIT 1`) so the result is deterministic even if
//! more than one row is left marked published.

use nebula_storage_port::dto::{WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::{WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{PgPool, Row};

use super::execution::conn_err;

/// Postgres-backed workflow-row store.
#[derive(Clone, Debug)]
pub struct PgWorkflowStore {
    pool: PgPool,
}

impl PgWorkflowStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl WorkflowStore for PgWorkflowStore {
    async fn create(&self, scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_workflows \
             (id, workspace_id, org_id, version, slug, deleted) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&record.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(record.version as i64)
        .bind(&record.slug)
        .bind(record.deleted)
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "workflow",
                    detail: format!("workflow {} already exists", record.id),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<WorkflowRecord>, StorageError> {
        // A soft-deleted row is a read miss, matching the other backends.
        let row = sqlx::query(
            "SELECT version, slug FROM port_workflows \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3 AND deleted = FALSE",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(row.map(|r| WorkflowRecord {
            id: id.to_string(),
            scope: scope.clone(),
            version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
            slug: r.try_get("slug").unwrap_or_default(),
            deleted: false,
        }))
    }

    async fn get_by_slug(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WorkflowRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT id, version FROM port_workflows \
             WHERE workspace_id = $1 AND org_id = $2 AND slug = $3 AND deleted = FALSE",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(row.map(|r| WorkflowRecord {
            id: r.try_get("id").unwrap_or_default(),
            scope: scope.clone(),
            version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
            slug: slug.to_string(),
            deleted: false,
        }))
    }

    async fn update(
        &self,
        scope: &Scope,
        record: WorkflowRecord,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        // CAS in one statement: the row is rewritten only when the stored
        // version still equals `expected_version`. Zero rows affected then
        // means either the row is gone (NotFound) or the version moved
        // (Conflict) — disambiguated by a follow-up read.
        let res = sqlx::query(
            "UPDATE port_workflows \
             SET version = $1, slug = $2, deleted = $3 \
             WHERE id = $4 AND workspace_id = $5 AND org_id = $6 AND version = $7",
        )
        .bind(record.version as i64)
        .bind(&record.slug)
        .bind(record.deleted)
        .bind(&record.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(expected_version as i64)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM port_workflows \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3",
        )
        .bind(&record.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        match current {
            Some(actual) => Err(StorageError::Conflict {
                entity: "workflow",
                id: record.id,
                expected: expected_version,
                actual: actual as u64,
            }),
            None => Err(StorageError::not_found("workflow", record.id)),
        }
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_workflows SET deleted = TRUE \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3 AND deleted = FALSE",
        )
        .bind(id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        if res.rows_affected() > 0 {
            Ok(())
        } else {
            Err(StorageError::not_found("workflow", id))
        }
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
        // Stable order by id so list output is deterministic across runs.
        let rows = sqlx::query(
            "SELECT id, version, slug FROM port_workflows \
             WHERE workspace_id = $1 AND org_id = $2 AND deleted = FALSE \
             ORDER BY id",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(rows
            .into_iter()
            .map(|r| WorkflowRecord {
                id: r.try_get("id").unwrap_or_default(),
                scope: scope.clone(),
                version: r.try_get::<i64, _>("version").unwrap_or_default() as u64,
                slug: r.try_get("slug").unwrap_or_default(),
                deleted: false,
            })
            .collect())
    }
}

/// Postgres-backed workflow-version store.
#[derive(Clone, Debug)]
pub struct PgWorkflowVersionStore {
    pool: PgPool,
}

impl PgWorkflowVersionStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Decode one version row. `definition` is a JSONB column mapped directly
/// to `serde_json::Value` by sqlx.
fn version_from_row(row: &sqlx::postgres::PgRow) -> Result<WorkflowVersionRecord, StorageError> {
    Ok(WorkflowVersionRecord {
        workflow_id: row.try_get("workflow_id").map_err(conn_err)?,
        number: row.try_get::<i64, _>("number").map_err(conn_err)? as u32,
        published: row.try_get("published").map_err(conn_err)?,
        pinned: row.try_get("pinned").map_err(conn_err)?,
        definition: row.try_get("definition").map_err(conn_err)?,
    })
}

#[async_trait::async_trait]
impl WorkflowVersionStore for PgWorkflowVersionStore {
    async fn create(
        &self,
        scope: &Scope,
        record: WorkflowVersionRecord,
    ) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_workflow_versions \
             (workspace_id, org_id, workflow_id, number, published, pinned, definition) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&record.workflow_id)
        .bind(i64::from(record.number))
        .bind(record.published)
        .bind(record.pinned)
        .bind(&record.definition)
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(StorageError::Duplicate {
                    entity: "workflow_version",
                    detail: format!(
                        "workflow {} version {} already exists",
                        record.workflow_id, record.number
                    ),
                })
            },
            Err(e) => Err(conn_err(e)),
        }
    }

    async fn get(
        &self,
        scope: &Scope,
        workflow_id: &str,
        number: u32,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT workflow_id, number, published, pinned, definition \
             FROM port_workflow_versions \
             WHERE workspace_id = $1 AND org_id = $2 AND workflow_id = $3 AND number = $4",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(workflow_id)
        .bind(i64::from(number))
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(version_from_row).transpose()
    }

    async fn get_published(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        // Highest-numbered published version wins (deterministic even if a
        // stale publish was left set on an older version).
        let row = sqlx::query(
            "SELECT workflow_id, number, published, pinned, definition \
             FROM port_workflow_versions \
             WHERE workspace_id = $1 AND org_id = $2 AND workflow_id = $3 \
               AND published = TRUE \
             ORDER BY number DESC LIMIT 1",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.as_ref().map(version_from_row).transpose()
    }

    async fn list(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<WorkflowVersionRecord>, StorageError> {
        // Newest first (highest version number first).
        let rows = sqlx::query(
            "SELECT workflow_id, number, published, pinned, definition \
             FROM port_workflow_versions \
             WHERE workspace_id = $1 AND org_id = $2 AND workflow_id = $3 \
             ORDER BY number DESC",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.iter().map(version_from_row).collect()
    }
}
