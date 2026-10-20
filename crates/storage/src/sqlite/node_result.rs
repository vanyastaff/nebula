//! SQLite node-result store (`0013_node_results.sql`).
//!
//! Three tables back the engine's resume seam: `node_outputs` (the raw
//! payload a node emitted), `node_results` (its typed result record) and
//! `workflow_inputs` (the execution's input). Every row belongs to its
//! execution — a write for an execution this database does not hold is
//! `NotFound`, and purging the execution removes its rows — and is keyed by
//! the execution's tenant, so a read under another scope never sees it.
//!
//! Each save is one upsert statement: the latest write of a slot wins. A
//! record whose schema version this binary cannot decode is refused on save
//! and fails closed on load with [`StorageError::UnknownSchemaVersion`].
//! Payloads never cross into an error or a span.

use nebula_storage_port::dto::NodeResultRecord;
use nebula_storage_port::store::NodeResultStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::node_result::{NodeSlot, guard_schema, stored_record};
use crate::sql_error::{foreign_key_not_found, storage_error};

/// SQLite-backed node-output / node-result store.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`]: the
/// deployment pool the execution store uses, since every row references its
/// execution.
#[derive(Clone, Debug)]
pub struct SqliteNodeResultStore {
    pool: SqlitePool,
}

impl SqliteNodeResultStore {
    /// Wrap an existing pool. The caller installs the port schema (see
    /// [`super::init_schema`]).
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// The upsert of `slot`: the latest write of a node's slot wins.
const fn upsert_sql(slot: NodeSlot) -> &'static str {
    match slot {
        NodeSlot::Output => {
            "INSERT INTO node_outputs \
                 (org_id, workspace_id, execution_id, node_key, kind_tag, payload, schema_version) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (org_id, workspace_id, execution_id, node_key) DO UPDATE SET \
                 kind_tag = excluded.kind_tag, payload = excluded.payload, \
                 schema_version = excluded.schema_version"
        },
        NodeSlot::Result => {
            "INSERT INTO node_results \
                 (org_id, workspace_id, execution_id, node_key, kind_tag, payload, schema_version) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (org_id, workspace_id, execution_id, node_key) DO UPDATE SET \
                 kind_tag = excluded.kind_tag, payload = excluded.payload, \
                 schema_version = excluded.schema_version"
        },
    }
}

/// The single-node read of `slot`.
const fn load_sql(slot: NodeSlot) -> &'static str {
    match slot {
        NodeSlot::Output => {
            "SELECT kind_tag, payload, schema_version FROM node_outputs \
                 WHERE org_id = ? AND workspace_id = ? AND execution_id = ? AND node_key = ?"
        },
        NodeSlot::Result => {
            "SELECT kind_tag, payload, schema_version FROM node_results \
                 WHERE org_id = ? AND workspace_id = ? AND execution_id = ? AND node_key = ?"
        },
    }
}

/// The per-execution read of `slot`, in node-key byte order.
const fn load_all_sql(slot: NodeSlot) -> &'static str {
    match slot {
        NodeSlot::Output => {
            "SELECT node_key, kind_tag, payload, schema_version FROM node_outputs \
                 WHERE org_id = ? AND workspace_id = ? AND execution_id = ? ORDER BY node_key"
        },
        NodeSlot::Result => {
            "SELECT node_key, kind_tag, payload, schema_version FROM node_results \
                 WHERE org_id = ? AND workspace_id = ? AND execution_id = ? ORDER BY node_key"
        },
    }
}

/// Decode one `(kind_tag, payload, schema_version)` row.
fn decode_node_result_record(row: &SqliteRow) -> Result<NodeResultRecord, StorageError> {
    let payload: String = row.try_get("payload").map_err(storage_error)?;
    let json = serde_json::from_str(&payload)
        .map_err(|_| StorageError::Corrupt("column `payload` is not the expected JSON".into()))?;
    stored_record(
        row.try_get("kind_tag").map_err(storage_error)?,
        json,
        row.try_get("schema_version").map_err(storage_error)?,
    )
}

/// Encode a record for its payload and schema-version columns, refusing a
/// schema version this binary cannot read back.
fn encode_record(record: &NodeResultRecord) -> Result<(String, i64), StorageError> {
    guard_schema(record)?;
    Ok((
        serde_json::to_string(&record.json)?,
        i64::from(record.schema_version),
    ))
}

#[tracing::instrument(
    level = "debug",
    name = "node_result.save",
    skip_all,
    fields(backend = "sqlite", slot = slot.label())
)]
async fn save_slot(
    pool: &SqlitePool,
    slot: NodeSlot,
    scope: &Scope,
    execution_id: &str,
    node_id: &str,
    record: &NodeResultRecord,
) -> Result<(), StorageError> {
    let (payload, schema_version) = encode_record(record)?;
    sqlx::query(upsert_sql(slot))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(execution_id)
        .bind(node_id)
        .bind(&record.kind_tag)
        .bind(payload)
        .bind(schema_version)
        .execute(pool)
        .await
        .map_err(|error| foreign_key_not_found(error, "execution", execution_id))?;
    Ok(())
}

#[tracing::instrument(
    level = "debug",
    name = "node_result.load",
    skip_all,
    fields(backend = "sqlite", slot = slot.label())
)]
async fn load_slot(
    pool: &SqlitePool,
    slot: NodeSlot,
    scope: &Scope,
    execution_id: &str,
    node_id: &str,
) -> Result<Option<NodeResultRecord>, StorageError> {
    sqlx::query(load_sql(slot))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(execution_id)
        .bind(node_id)
        .fetch_optional(pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_node_result_record)
        .transpose()
}

#[tracing::instrument(
    level = "debug",
    name = "node_result.load_all",
    skip_all,
    fields(backend = "sqlite", slot = slot.label())
)]
async fn load_all_slots(
    pool: &SqlitePool,
    slot: NodeSlot,
    scope: &Scope,
    execution_id: &str,
) -> Result<Vec<(String, NodeResultRecord)>, StorageError> {
    let rows = sqlx::query(load_all_sql(slot))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(execution_id)
        .fetch_all(pool)
        .await
        .map_err(storage_error)?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("node_key").map_err(storage_error)?,
                decode_node_result_record(row)?,
            ))
        })
        .collect()
}

#[async_trait::async_trait]
impl NodeResultStore for SqliteNodeResultStore {
    async fn save_node_output(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        output: NodeResultRecord,
    ) -> Result<(), StorageError> {
        save_slot(
            &self.pool,
            NodeSlot::Output,
            scope,
            execution_id,
            node_id,
            &output,
        )
        .await
    }

    async fn load_node_output(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        load_slot(&self.pool, NodeSlot::Output, scope, execution_id, node_id).await
    }

    async fn save_node_result(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        result: NodeResultRecord,
    ) -> Result<(), StorageError> {
        save_slot(
            &self.pool,
            NodeSlot::Result,
            scope,
            execution_id,
            node_id,
            &result,
        )
        .await
    }

    async fn load_node_result(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        load_slot(&self.pool, NodeSlot::Result, scope, execution_id, node_id).await
    }

    async fn load_all_results(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError> {
        load_all_slots(&self.pool, NodeSlot::Result, scope, execution_id).await
    }

    async fn load_all_node_outputs(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<(String, NodeResultRecord)>, StorageError> {
        load_all_slots(&self.pool, NodeSlot::Output, scope, execution_id).await
    }

    #[tracing::instrument(
        level = "debug",
        name = "node_result.set_workflow_input",
        skip_all,
        fields(backend = "sqlite")
    )]
    async fn set_workflow_input(
        &self,
        scope: &Scope,
        execution_id: &str,
        input: NodeResultRecord,
    ) -> Result<(), StorageError> {
        let (payload, schema_version) = encode_record(&input)?;
        sqlx::query(
            "INSERT INTO workflow_inputs \
             (org_id, workspace_id, execution_id, kind_tag, payload, schema_version) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (org_id, workspace_id, execution_id) DO UPDATE SET \
             kind_tag = excluded.kind_tag, payload = excluded.payload, \
             schema_version = excluded.schema_version",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(execution_id)
        .bind(&input.kind_tag)
        .bind(payload)
        .bind(schema_version)
        .execute(&self.pool)
        .await
        .map_err(|error| foreign_key_not_found(error, "execution", execution_id))?;
        Ok(())
    }

    #[tracing::instrument(
        level = "debug",
        name = "node_result.get_workflow_input",
        skip_all,
        fields(backend = "sqlite")
    )]
    async fn get_workflow_input(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<NodeResultRecord>, StorageError> {
        sqlx::query(
            "SELECT kind_tag, payload, schema_version FROM workflow_inputs \
             WHERE org_id = ? AND workspace_id = ? AND execution_id = ?",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_node_result_record)
        .transpose()
    }
}
