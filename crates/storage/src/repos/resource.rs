//! Resource repository.

use async_trait::async_trait;

use crate::error::StorageError;

/// Workspace-scoped resource metadata.
///
/// Spec 16 layer 6. Resources are long-lived managed objects
/// (connection pools, SDK clients). The engine owns lifecycle; this
/// repo only stores definitions.
#[async_trait]
pub trait ResourceRepo: Send + Sync {
    /// Insert a new resource definition.
    async fn create(&self, resource: &ResourceEntry) -> Result<(), StorageError>;

    /// Fetch a resource by ID.
    async fn get(&self, id: &[u8]) -> Result<Option<ResourceEntry>, StorageError>;

    /// Fetch a resource by (workspace_id, slug).
    async fn get_by_slug(
        &self,
        workspace_id: &[u8],
        slug: &str,
    ) -> Result<Option<ResourceEntry>, StorageError>;

    /// Update a resource with CAS on `version`.
    async fn update(
        &self,
        resource: &ResourceEntry,
        expected_version: i64,
    ) -> Result<(), StorageError>;

    /// Soft-delete a resource.
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    /// List all resources in a workspace.
    async fn list(&self, workspace_id: &[u8]) -> Result<Vec<ResourceEntry>, StorageError>;
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
