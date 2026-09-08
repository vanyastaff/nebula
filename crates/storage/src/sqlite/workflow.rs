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
use nebula_storage_port::store::{WorkflowPublicationError, WorkflowStore, WorkflowVersionStore};
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
        let next =
            i64::try_from(row.version).map_err(|_| WorkflowPublicationError::InvalidPublication)?;
        let expected = i64::try_from(expected_version)
            .map_err(|_| WorkflowPublicationError::InvalidPublication)?;
        let definition = serde_json::to_string(&version.definition).map_err(StorageError::from)?;
        let activation_json = serde_json::to_string(&activation).map_err(StorageError::from)?;
        let mut transaction = self.pool.begin().await.map_err(conn_err)?;
        let changed = sqlx::query("UPDATE port_workflows SET version = ?, slug = ? WHERE id = ? AND workspace_id = ? AND org_id = ? AND version = ? AND deleted = 0")
            .bind(next).bind(&row.slug).bind(&row.id).bind(&scope.workspace_id).bind(&scope.org_id).bind(expected)
            .execute(&mut *transaction).await.map_err(conn_err)?.rows_affected();
        if changed != 1 {
            let actual = sqlx::query_scalar::<_, i64>("SELECT version FROM port_workflows WHERE id = ? AND workspace_id = ? AND org_id = ? AND deleted = 0")
                .bind(&row.id).bind(&scope.workspace_id).bind(&scope.org_id)
                .fetch_optional(&mut *transaction).await.map_err(conn_err)?;
            return Err(match actual {
                Some(actual) => StorageError::Conflict {
                    entity: "workflow",
                    id: row.id,
                    expected: expected_version,
                    actual: actual
                        .try_into()
                        .map_err(|_| WorkflowPublicationError::InvalidPublication)?,
                },
                None => StorageError::not_found("workflow", row.id),
            }
            .into());
        }
        let ids = activation.revisions();
        let plan: Option<Vec<u8>> = sqlx::query_scalar("SELECT p.record_bytes FROM port_executable_plan_revisions p JOIN port_worker_flavor_revisions f ON f.worker_flavor_id = p.worker_flavor_id WHERE p.executable_plan_id = ? AND p.worker_flavor_id = ? AND p.lifecycle = 'active' AND f.lifecycle = 'active'")
            .bind(ids.plan().as_bytes().as_slice()).bind(ids.worker_flavor().as_bytes().as_slice())
            .fetch_optional(&mut *transaction).await.map_err(conn_err)?;
        let plan = plan.ok_or(WorkflowPublicationError::RevisionNotAdmitted)?;
        crate::workflow_activation::validate_plan_identity(&plan, &row.id, activation)?;
        let reused: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_workflow_versions WHERE workspace_id = ? AND org_id = ? AND workflow_id = ? AND json_extract(activation, '$.workflow_version_id') = ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(&row.id).bind(activation.workflow_version_id().to_string())
            .fetch_one(&mut *transaction).await.map_err(conn_err)?;
        if reused != 0 {
            return Err(WorkflowPublicationError::InvalidPublication);
        }
        sqlx::query("INSERT INTO port_workflow_versions (workspace_id, org_id, workflow_id, number, published, pinned, definition, activation) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(&version.workflow_id).bind(i64::from(version.number))
            .bind(i64::from(version.published)).bind(i64::from(version.pinned)).bind(definition).bind(activation_json)
            .execute(&mut *transaction).await.map_err(|error| match error {
                sqlx::Error::Database(database) if database.is_unique_violation() =>
                    StorageError::Duplicate { entity: "workflow_version", detail: "workflow version already exists".into() },
                other => conn_err(other),
            })?;
        transaction
            .commit()
            .await
            .map_err(|_| WorkflowPublicationError::OutcomeUnknown)?;
        Ok(())
    }

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
        // version still equals `expected_version` AND it is not a
        // tombstone. `deleted = 0` is mandatory — without it an `update`
        // on a soft-deleted row would rewrite it (clearing the tombstone)
        // and resurrect a row that `get`/`get_by_slug`/`list` already
        // treat as gone. Zero rows affected then means the row is
        // gone/tombstoned (NotFound) or the version moved (Conflict) —
        // disambiguated by a follow-up read.
        let res = sqlx::query(
            "UPDATE port_workflows \
             SET version = ?, slug = ?, deleted = ? \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND version = ? AND deleted = 0",
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
        // Disambiguate behind the same tombstone-invisible predicate the
        // UPDATE used: a soft-deleted row must surface as `NotFound`
        // (a read miss, matching `get`), never a spurious `Conflict`.
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM port_workflows \
             WHERE id = ? AND workspace_id = ? AND org_id = ? \
               AND deleted = 0",
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

    async fn save_with_published_version(
        &self,
        scope: &Scope,
        row: WorkflowRecord,
        version: WorkflowVersionRecord,
        expected_version: Option<u64>,
    ) -> Result<(), StorageError> {
        if version.activation.is_some() {
            return Err(StorageError::Internal(
                "activated versions require publication admission".into(),
            ));
        }
        let def = serde_json::to_string(&version.definition)?;
        // One transaction so the row write and the version write commit
        // (or roll back) together — no orphan-row window.
        let mut tx = self.pool.begin().await.map_err(conn_err)?;

        match expected_version {
            None => {
                // Create the row.
                let res = sqlx::query(
                    "INSERT INTO port_workflows \
                     (id, workspace_id, org_id, version, slug, deleted) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&row.id)
                .bind(&scope.workspace_id)
                .bind(&scope.org_id)
                .bind(row.version as i64)
                .bind(&row.slug)
                .bind(i64::from(row.deleted))
                .execute(&mut *tx)
                .await;
                if let Err(e) = res {
                    return Err(match e {
                        sqlx::Error::Database(db) if db.is_unique_violation() => {
                            StorageError::Duplicate {
                                entity: "workflow",
                                detail: format!("workflow {} already exists", row.id),
                            }
                        },
                        other => conn_err(other),
                    });
                }
            },
            Some(expected) => {
                // CAS the row counter forward in the same tx.
                let res = sqlx::query(
                    "UPDATE port_workflows \
                     SET version = ?, slug = ?, deleted = ? \
                     WHERE id = ? AND workspace_id = ? AND org_id = ? AND version = ?",
                )
                .bind(row.version as i64)
                .bind(&row.slug)
                .bind(i64::from(row.deleted))
                .bind(&row.id)
                .bind(&scope.workspace_id)
                .bind(&scope.org_id)
                .bind(expected as i64)
                .execute(&mut *tx)
                .await
                .map_err(conn_err)?;
                if res.rows_affected() == 0 {
                    // Disambiguate row-gone vs version-moved within the tx
                    // (rolled back on drop) so neither write lands.
                    let current = sqlx::query_scalar::<_, i64>(
                        "SELECT version FROM port_workflows \
                         WHERE id = ? AND workspace_id = ? AND org_id = ?",
                    )
                    .bind(&row.id)
                    .bind(&scope.workspace_id)
                    .bind(&scope.org_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(conn_err)?;
                    return Err(match current {
                        Some(actual) => StorageError::Conflict {
                            entity: "workflow",
                            id: row.id,
                            expected,
                            actual: actual as u64,
                        },
                        None => StorageError::not_found("workflow", row.id),
                    });
                }
            },
        }

        // Append the published version row inside the same tx.
        let res = sqlx::query(
            "INSERT INTO port_workflow_versions \
             (workspace_id, org_id, workflow_id, number, published, pinned, definition) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&version.workflow_id)
        .bind(i64::from(version.number))
        .bind(i64::from(version.published))
        .bind(i64::from(version.pinned))
        .bind(&def)
        .execute(&mut *tx)
        .await;
        if let Err(e) = res {
            return Err(match e {
                sqlx::Error::Database(db) if db.is_unique_violation() => StorageError::Duplicate {
                    entity: "workflow_version",
                    detail: format!(
                        "workflow {} version {} already exists",
                        version.workflow_id, version.number
                    ),
                },
                other => conn_err(other),
            });
        }

        tx.commit().await.map_err(conn_err)?;
        Ok(())
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

    async fn count(&self, scope: &Scope) -> Result<u64, StorageError> {
        // Same active-in-scope predicate as `list`, answered with
        // COUNT(*) so callers on the hot path never materialize rows.
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM port_workflows \
             WHERE workspace_id = ? AND org_id = ? AND deleted = 0",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_one(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(n.max(0) as u64)
    }

    async fn is_reachable(&self) -> Result<(), StorageError> {
        // Cheapest possible liveness round-trip: no table touched, no
        // tenant predicate. Any pool/transport error maps to the
        // `StorageError` the readiness probe treats as "not ready".
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(conn_err)?;
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

/// Decode one version row. The `definition` column is opaque JSON text.
fn version_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<WorkflowVersionRecord, StorageError> {
    let def_str: String = row.try_get("definition").map_err(conn_err)?;
    let definition: serde_json::Value = serde_json::from_str(&def_str)?;
    Ok(WorkflowVersionRecord {
        activation: row
            .try_get::<Option<String>, _>("activation")
            .map_err(conn_err)?
            .map(|encoded| serde_json::from_str(&encoded))
            .transpose()?,
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
        if record.activation.is_some() {
            return Err(StorageError::Internal(
                "activated versions require publication admission".into(),
            ));
        }
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
            "SELECT workflow_id, number, published, pinned, definition, activation \
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
            "SELECT workflow_id, number, published, pinned, definition, activation \
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
            "SELECT workflow_id, number, published, pinned, definition, activation \
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
