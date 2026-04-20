//! Resource repository.

use std::future::Future;

use crate::error::StorageError;

/// Workspace-scoped resource metadata.
///
/// Spec 16 layer 6. Resources are long-lived managed objects
/// (connection pools, SDK clients). The engine owns lifecycle; this
/// repo only stores definitions.
pub trait ResourceRepo: Send + Sync {
    /// Insert a new resource definition.
    fn create(
        &self,
        resource: &ResourceEntry,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Fetch a resource by ID.
    fn get(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<ResourceEntry>, StorageError>> + Send;

    /// Fetch a resource by (workspace_id, slug).
    fn get_by_slug(
        &self,
        workspace_id: &[u8],
        slug: &str,
    ) -> impl Future<Output = Result<Option<ResourceEntry>, StorageError>> + Send;

    /// Update a resource with CAS on `version`.
    fn update(
        &self,
        resource: &ResourceEntry,
        expected_version: i64,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Soft-delete a resource.
    fn soft_delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List all resources in a workspace.
    fn list(
        &self,
        workspace_id: &[u8],
    ) -> impl Future<Output = Result<Vec<ResourceEntry>, StorageError>> + Send;
}

/// Resource row (in-repo type — the `resources` table is simple enough
/// to keep the entry adjacent to the trait).
#[derive(Debug, Clone)]
pub struct ResourceEntry {
    /// `res_` ULID, 16-byte BYTEA.
    pub id: Vec<u8>,
    /// Owning workspace.
    pub workspace_id: Vec<u8>,
    /// Workspace-unique slug.
    pub slug: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Resource kind key (e.g. `'postgres_pool'`, `'http_client'`).
    pub kind: String,
    /// Resource-specific configuration.
    pub config: serde_json::Value,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Creator principal.
    pub created_by: Vec<u8>,
    /// CAS version.
    pub version: i64,
    /// Soft-delete timestamp.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}
