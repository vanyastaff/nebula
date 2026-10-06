//! SQLite `WorkflowStore` + `WorkflowVersionStore` over `workflows` and
//! `workflow_versions`.
//!
//! The workflow row (id / slug / soft delete / CAS version) and its versions
//! (each carrying the opaque definition payload) are separate tables. Every
//! query carries `WHERE org_id = ? AND workspace_id = ?`, so a cross-tenant
//! `get` yields `Ok(None)` and a cross-tenant `update` / `soft_delete` is
//! `NotFound` — an id outside the caller's scope is indistinguishable from
//! one that does not exist (no existence oracle). A soft-deleted workflow is
//! invisible to every read and write.
//!
//! `get_published` returns the **highest-numbered** published version, so
//! the result is deterministic while older versions stay marked published.

use nebula_storage_port::dto::{WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::{WorkflowPublicationError, WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{Scope, StorageError};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::sql_error::{decode_u64, encode_u64, storage_error, storage_error_for};
use crate::workflow_activation::ActivationColumns;

/// Versions of live workflows only: a soft-deleted workflow's versions are
/// invisible with it.
const LIVE_VERSIONS: &str = "SELECT v.workflow_id, v.number, v.published, v.pinned, \
     v.definition, v.activation_workflow_version_id, v.activation_executable_plan_id, \
     v.activation_worker_flavor_id \
     FROM workflow_versions v JOIN workflows w ON w.org_id = v.org_id \
       AND w.workspace_id = v.workspace_id AND w.id = v.workflow_id AND w.deleted_at IS NULL";

/// Decode one live `workflows` row selected as `id, version, slug`.
fn decode_workflow(row: &SqliteRow, scope: &Scope) -> Result<WorkflowRecord, StorageError> {
    Ok(WorkflowRecord {
        id: row.try_get("id").map_err(storage_error)?,
        scope: scope.clone(),
        version: decode_u64(row.try_get("version").map_err(storage_error)?, "version")?,
        slug: row.try_get("slug").map_err(storage_error)?,
    })
}

fn decode_version(row: &SqliteRow) -> Result<WorkflowVersionRecord, StorageError> {
    let activation = ActivationColumns {
        workflow_version: row
            .try_get("activation_workflow_version_id")
            .map_err(storage_error)?,
        executable_plan: row
            .try_get("activation_executable_plan_id")
            .map_err(storage_error)?,
        worker_flavor: row
            .try_get("activation_worker_flavor_id")
            .map_err(storage_error)?,
    };
    let definition = row
        .try_get::<String, _>("definition")
        .map_err(storage_error)?;
    Ok(WorkflowVersionRecord {
        activation: activation.decode()?,
        workflow_id: row.try_get("workflow_id").map_err(storage_error)?,
        number: u32::try_from(row.try_get::<i64, _>("number").map_err(storage_error)?).map_err(
            |_| StorageError::Corrupt("column `number` is outside the u32 range".into()),
        )?,
        published: row.try_get::<i64, _>("published").map_err(storage_error)? != 0,
        pinned: row.try_get::<i64, _>("pinned").map_err(storage_error)? != 0,
        definition: serde_json::from_str(&definition).map_err(|_| {
            StorageError::Corrupt("column `definition` is not the expected JSON".into())
        })?,
    })
}

/// Insert a live workflow row inside the caller's `BEGIN IMMEDIATE`
/// transaction. A taken id or active slug is `Duplicate`; a missing or
/// deleted workspace is `NotFound` (the foreign key proves existence only).
async fn insert_workflow(
    connection: &mut SqliteConnection,
    scope: &Scope,
    row: &WorkflowRecord,
) -> Result<(), StorageError> {
    let workspace = sqlx::query(
        "SELECT w.id FROM workspaces w JOIN orgs o ON o.id = w.org_id \
         WHERE w.org_id = ? AND w.id = ? AND w.deleted_at IS NULL AND o.deleted_at IS NULL",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(storage_error)?;
    if workspace.is_none() {
        return Err(StorageError::not_found(
            "workspace",
            scope.workspace_id.clone(),
        ));
    }
    sqlx::query(
        "INSERT INTO workflows (org_id, workspace_id, id, slug, version) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&row.id)
    .bind(&row.slug)
    .bind(encode_u64(row.version, "version")?)
    .execute(connection)
    .await
    .map_err(|error| storage_error_for("workflow", error))?;
    Ok(())
}

/// CAS-rewrite a live workflow row; zero rows ⇒ `NotFound` or `Conflict`.
async fn update_workflow(
    connection: &mut SqliteConnection,
    scope: &Scope,
    row: &WorkflowRecord,
    expected_version: u64,
) -> Result<(), StorageError> {
    let changed = sqlx::query(
        "UPDATE workflows SET version = ?, slug = ? \
         WHERE org_id = ? AND workspace_id = ? AND id = ? \
           AND version = ? AND deleted_at IS NULL",
    )
    .bind(encode_u64(row.version, "version")?)
    .bind(&row.slug)
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&row.id)
    .bind(encode_u64(expected_version, "version")?)
    .execute(&mut *connection)
    .await
    .map_err(|error| storage_error_for("workflow", error))?
    .rows_affected();
    if changed > 0 {
        return Ok(());
    }
    // Disambiguate behind the same tombstone-invisible predicate: a deleted
    // row is `NotFound`, never a spurious `Conflict`.
    let current = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM workflows \
         WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&row.id)
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    match current {
        Some(actual) => Err(StorageError::Conflict {
            entity: "workflow",
            id: row.id.clone(),
            expected: expected_version,
            actual: decode_u64(actual, "version")?,
        }),
        None => Err(StorageError::not_found("workflow", row.id.clone())),
    }
}

/// Append one version row inside the caller's `BEGIN IMMEDIATE`
/// transaction. A taken number or activation identity is `Duplicate`; a
/// missing or deleted workflow is `NotFound`.
async fn insert_version(
    connection: &mut SqliteConnection,
    scope: &Scope,
    version: &WorkflowVersionRecord,
) -> Result<(), StorageError> {
    let workflow = sqlx::query(
        "SELECT id FROM workflows \
         WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&version.workflow_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(storage_error)?;
    if workflow.is_none() {
        return Err(StorageError::not_found(
            "workflow",
            version.workflow_id.clone(),
        ));
    }
    let activation = ActivationColumns::encode(version.activation);
    sqlx::query(
        "INSERT INTO workflow_versions (org_id, workspace_id, workflow_id, number, \
         published, pinned, definition, activation_workflow_version_id, \
         activation_executable_plan_id, activation_worker_flavor_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(&version.workflow_id)
    .bind(i64::from(version.number))
    .bind(i64::from(version.published))
    .bind(i64::from(version.pinned))
    .bind(version.definition.to_string())
    .bind(activation.workflow_version)
    .bind(activation.executable_plan)
    .bind(activation.worker_flavor)
    .execute(connection)
    .await
    .map_err(|error| storage_error_for("workflow_version", error))?;
    Ok(())
}

fn require_unactivated(version: &WorkflowVersionRecord) -> Result<(), StorageError> {
    if version.activation.is_some() {
        return Err(StorageError::InvalidInput(
            "activated versions require publication admission".into(),
        ));
    }
    Ok(())
}

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
    #[tracing::instrument(skip_all, fields(workflow_id = %row.id, expected_version), err)]
    async fn publish_activated_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: u64,
    ) -> Result<(), WorkflowPublicationError> {
        let activation = crate::workflow_activation::validate_publication(
            scope,
            &row,
            &version,
            expected_version,
        )?;
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        update_workflow(&mut transaction, scope, &row, expected_version).await?;
        let ids = activation.revisions();
        let plan: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT p.record_bytes FROM executable_plan_revisions p \
             JOIN worker_flavor_revisions f ON f.worker_flavor_id = p.worker_flavor_id \
             WHERE p.executable_plan_id = ? AND p.worker_flavor_id = ? \
               AND p.lifecycle = 'active' AND f.lifecycle = 'active'",
        )
        .bind(ids.plan().as_bytes().as_slice())
        .bind(ids.worker_flavor().as_bytes().as_slice())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_error)?;
        let plan = plan.ok_or(WorkflowPublicationError::RevisionNotAdmitted)?;
        crate::workflow_activation::validate_plan_identity(&plan, &row.id, activation)?;
        // An activation identity is unique across all workflows; reusing one
        // is a disagreeing publication, not a storage collision.
        let reused =
            sqlx::query("SELECT 1 FROM workflow_versions WHERE activation_workflow_version_id = ?")
                .bind(activation.workflow_version_id().to_string())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(storage_error)?;
        if reused.is_some() {
            return Err(WorkflowPublicationError::InvalidPublication);
        }
        insert_version(&mut transaction, scope, &version).await?;
        transaction
            .commit()
            .await
            .map_err(|_| WorkflowPublicationError::OutcomeUnknown)?;
        Ok(())
    }

    async fn create(&self, scope: &Scope, record: WorkflowRecord) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        insert_workflow(&mut tx, scope, &record).await?;
        tx.commit().await.map_err(storage_error)
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<WorkflowRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT id, version, slug FROM workflows \
             WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        row.map(|row| decode_workflow(&row, scope)).transpose()
    }

    async fn get_by_slug(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WorkflowRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT id, version, slug FROM workflows \
             WHERE org_id = ? AND workspace_id = ? AND slug = ? AND deleted_at IS NULL",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        row.map(|row| decode_workflow(&row, scope)).transpose()
    }

    async fn update(
        &self,
        scope: &Scope,
        record: WorkflowRecord,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let mut connection = self.pool.acquire().await.map_err(storage_error)?;
        update_workflow(&mut connection, scope, &record, expected_version).await
    }

    async fn save_with_published_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: Option<u64>,
    ) -> Result<(), StorageError> {
        require_unactivated(&version)?;
        // One transaction: the row write and the version write commit (or
        // roll back) together — no orphan-row window.
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        match expected_version {
            None => insert_workflow(&mut tx, scope, &row).await?,
            Some(expected) => update_workflow(&mut tx, scope, &row, expected).await?,
        }
        insert_version(&mut tx, scope, &version).await?;
        tx.commit().await.map_err(storage_error)
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE workflows SET deleted_at = ? \
             WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(chrono::Utc::now().timestamp_micros())
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        if res.rows_affected() > 0 {
            Ok(())
        } else {
            Err(StorageError::not_found("workflow", id))
        }
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<WorkflowRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, version, slug FROM workflows \
             WHERE org_id = ? AND workspace_id = ? AND deleted_at IS NULL ORDER BY id",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        rows.iter().map(|row| decode_workflow(row, scope)).collect()
    }

    async fn count(&self, scope: &Scope) -> Result<u64, StorageError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflows \
             WHERE org_id = ? AND workspace_id = ? AND deleted_at IS NULL",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;
        decode_u64(n, "count")
    }

    async fn is_reachable(&self) -> Result<(), StorageError> {
        // Cheapest liveness round-trip: no table, no tenant predicate.
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(storage_error)?;
        Ok(())
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

#[async_trait::async_trait]
impl WorkflowVersionStore for SqliteWorkflowVersionStore {
    async fn create(
        &self,
        scope: &Scope,
        record: WorkflowVersionRecord,
    ) -> Result<(), StorageError> {
        require_unactivated(&record)?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        insert_version(&mut tx, scope, &record).await?;
        tx.commit().await.map_err(storage_error)
    }

    async fn get(
        &self,
        scope: &Scope,
        workflow_id: &str,
        number: u32,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        let sql = format!(
            "{LIVE_VERSIONS} \
             WHERE v.org_id = ? AND v.workspace_id = ? AND v.workflow_id = ? AND v.number = ?"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(workflow_id)
            .bind(i64::from(number))
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_version)
            .transpose()
    }

    async fn get_published(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Option<WorkflowVersionRecord>, StorageError> {
        let sql = format!(
            "{LIVE_VERSIONS} \
             WHERE v.org_id = ? AND v.workspace_id = ? AND v.workflow_id = ? AND v.published = 1 \
             ORDER BY v.number DESC LIMIT 1"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(workflow_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_version)
            .transpose()
    }

    async fn list(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<WorkflowVersionRecord>, StorageError> {
        let sql = format!(
            "{LIVE_VERSIONS} \
             WHERE v.org_id = ? AND v.workspace_id = ? AND v.workflow_id = ? \
             ORDER BY v.number DESC"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(workflow_id)
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?
            .iter()
            .map(decode_version)
            .collect()
    }
}
