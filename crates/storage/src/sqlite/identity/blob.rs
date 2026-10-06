//! `port_blobs`: workspace-scoped payloads with an optional expiry.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::BlobRow;
use nebula_storage_port::store::BlobStore;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;

use super::{json_text, now_rfc3339, optional, optional_json, required};
use crate::sql_error::storage_error;

/// SQLite-backed `blobs` store.
#[derive(Clone, Debug)]
pub struct SqliteBlobStore {
    pool: SqlitePool,
}

impl SqliteBlobStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_blob(row: &SqliteRow) -> Result<BlobRow, StorageError> {
    Ok(BlobRow {
        id: required(row, "id")?,
        workspace_id: required(row, "workspace_id")?,
        execution_id: optional(row, "execution_id")?,
        kind: required(row, "kind")?,
        content_type: optional(row, "content_type")?,
        size_bytes: required(row, "size_bytes")?,
        checksum: optional(row, "checksum")?,
        storage_mode: required(row, "storage_mode")?,
        data: optional(row, "data")?,
        external_ref: optional(row, "external_ref")?,
        metadata: optional_json(row, "metadata")?,
        created_at: required(row, "created_at")?,
        expires_at: optional(row, "expires_at")?,
    })
}

#[async_trait::async_trait]
impl BlobStore for SqliteBlobStore {
    async fn put(&self, row: BlobRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_blobs (id, workspace_id, execution_id, kind, \
             content_type, size_bytes, checksum, storage_mode, data, \
             external_ref, metadata, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (workspace_id, id) DO UPDATE SET \
             execution_id = excluded.execution_id, kind = excluded.kind, \
             content_type = excluded.content_type, \
             size_bytes = excluded.size_bytes, checksum = excluded.checksum, \
             storage_mode = excluded.storage_mode, data = excluded.data, \
             external_ref = excluded.external_ref, metadata = excluded.metadata, \
             created_at = excluded.created_at, expires_at = excluded.expires_at",
        )
        .bind(&row.id)
        .bind(&row.workspace_id)
        .bind(&row.execution_id)
        .bind(&row.kind)
        .bind(&row.content_type)
        .bind(row.size_bytes)
        .bind(&row.checksum)
        .bind(&row.storage_mode)
        .bind(&row.data)
        .bind(&row.external_ref)
        .bind(row.metadata.as_ref().map(json_text))
        .bind(&row.created_at)
        .bind(&row.expires_at)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn get(&self, workspace_id: &str, id: &str) -> Result<Option<BlobRow>, StorageError> {
        sqlx::query("SELECT * FROM port_blobs WHERE workspace_id = ? AND id = ?")
            .bind(workspace_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_blob)
            .transpose()
    }

    async fn delete(&self, workspace_id: &str, id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM port_blobs WHERE workspace_id = ? AND id = ?")
            .bind(workspace_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        let res =
            sqlx::query("DELETE FROM port_blobs WHERE expires_at IS NOT NULL AND expires_at <= ?")
                .bind(now_rfc3339())
                .execute(&self.pool)
                .await
                .map_err(storage_error)?;
        Ok(res.rows_affected())
    }
}
