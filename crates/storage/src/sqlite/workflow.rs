//! SQLite `WorkflowStore` + `WorkflowVersionStore` (spec-16 split) over
//! the port-scoped schema.
//!
//! The workflow row (id / slug / soft-delete / CAS version) and its
//! versions (each carrying the opaque definition payload) are separate
//! tables. Every query carries `WHERE workspace_id = ? AND org_id = ?`, so
//! a cross-tenant `get` yields `Ok(None)` and a cross-tenant `update` /
//! `soft_delete` is a `NotFound` — an id outside the caller's scope is
//! indistinguishable from one that does not exist (no existence oracle),
//! exactly as the in-memory backend behaves.
//!
//! `get_published` returns the **highest-numbered** published version so
//! the result is deterministic if more than one row is (incorrectly) left
//! marked published — this matches the in-memory store's `max_by_key`.

use nebula_storage_port::dto::{WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::{WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqlitePool};

use super::execution::conn_err;

/// SQLite-backed workflow-row store.
#[derive(Clone, Debug)]
pub struct SqliteWorkflowStore {
    pool: SqlitePool,
}

impl SqliteWorkflowStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl WorkflowStore for SqliteWorkflowStore {
    async fn create(&self, scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError> {
        let res = sqlx::query(
            "INSERT INTO port_workflows \
             (id, workspace_id, org_id, version, slug, deleted) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(record.version as i64)
        .bind(&record.slug)
        .bind(i64::from(record.deleted))
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
        // A soft-deleted row is a read miss (callers needing tombstones
        // would use a future list variant), matching the in-memory store.
        let row = sqlx::query(
            "SELECT version, slug, deleted FROM port_workflows \
             WHERE id = ? AND workspace_id = ? AND org_id = ? AND deleted = 0",
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
             WHERE workspace_id = ? AND org_id = ? AND slug = ? AND deleted = 0",
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
             SET version = ?, slug = ?, deleted = ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ? AND version = ?",
        )
        .bind(record.version as i64)
        .bind(&record.slug)
        .bind(i64::from(record.deleted))
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
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
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
            "UPDATE port_workflows SET deleted = 1 \
             WHERE id = ? AND workspace_id = ? AND org_id = ? AND deleted = 0",
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
        // Stable order by id so list output is deterministic across runs,
        // matching the in-memory store.
        let rows = sqlx::query(
            "SELECT id, version, slug FROM port_workflows \
             WHERE workspace_id = ? AND org_id = ? AND deleted = 0 \
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

/// SQLite-backed workflow-version store.
#[derive(Clone, Debug)]
pub struct SqliteWorkflowVersionStore {
    pool: SqlitePool,
}

impl SqliteWorkflowVersionStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Decode one version row. The `definition` column is opaque JSON text.
fn version_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<WorkflowVersionRecord, StorageError> {
    let def_str: String = row.try_get("definition").map_err(conn_err)?;
    let definition: serde_json::Value = serde_json::from_str(&def_str)?;
    Ok(WorkflowVersionRecord {
        workflow_id: row.try_get("workflow_id").map_err(conn_err)?,
        number: row.try_get::<i64, _>("number").map_err(conn_err)? as u32,
        published: row.try_get::<i64, _>("published").map_err(conn_err)? != 0,
        pinned: row.try_get::<i64, _>("pinned").map_err(conn_err)? != 0,
        definition,
    })
}

#[async_trait::async_trait]
impl WorkflowVersionStore for SqliteWorkflowVersionStore {
    async fn create(
        &self,
        scope: &Scope,
        record: WorkflowVersionRecord,
    ) -> Result<(), StorageError> {
        let def = serde_json::to_string(&record.definition)?;
        let res = sqlx::query(
            "INSERT INTO port_workflow_versions \
             (workspace_id, org_id, workflow_id, number, published, pinned, definition) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&record.workflow_id)
        .bind(i64::from(record.number))
        .bind(i64::from(record.published))
        .bind(i64::from(record.pinned))
        .bind(&def)
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
             WHERE workspace_id = ? AND org_id = ? AND workflow_id = ? AND number = ?",
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
             WHERE workspace_id = ? AND org_id = ? AND workflow_id = ? \
               AND published = 1 \
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
        // Newest first (highest version number first), matching the
        // in-memory store.
        let rows = sqlx::query(
            "SELECT workflow_id, number, published, pinned, definition \
             FROM port_workflow_versions \
             WHERE workspace_id = ? AND org_id = ? AND workflow_id = ? \
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
