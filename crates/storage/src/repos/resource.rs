//! Resource repository.

use async_trait::async_trait;

use crate::error::StorageError;

/// Workspace-scoped resource metadata.
///
/// Spec 16 layer 6. Resources are long-lived managed objects
/// (connection pools, SDK clients). The engine owns lifecycle; this
/// repo only stores definitions.
///
/// `#[async_trait]` (not RPITIT / `async fn` in trait) is required for
/// object safety: the API layer holds this as
/// `Arc<dyn ResourceRepo>` in `AppState`, and an async-fn-in-trait
/// (RPITIT) trait is **not** `dyn`-compatible on stable Rust — its
/// methods desugar to an opaque `impl Future` return whose type cannot
/// be named through a vtable. `#[async_trait]` boxes the future
/// (`Pin<Box<dyn Future + Send>>`) so `dyn ResourceRepo` is object-safe.
/// This also matches the other `#[async_trait]` port repos the API
/// layer holds as `Arc<dyn …>` (`ControlQueueRepo`,
/// `WebhookActivationRepo`). `ResourceRepo` had no impls when written,
/// so the choice is non-breaking. (The sibling RPITIT
/// `repos::WorkflowRepo` / `repos::ExecutionRepo` are spec-16
/// planned/experimental and are *not* used as `dyn` anywhere — they are
/// not a precedent for a `dyn`-held RPITIT trait, which stable does not
/// permit.)
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

    /// Update a resource with CAS on `version`, returning the
    /// store-owned post-CAS version.
    ///
    /// CAS is checked on `expected_version`. The returned `i64` is the
    /// **authoritative** new version the store assigned after the
    /// compare-and-swap (the post-CAS value, i.e. `actual + 1`); it is
    /// what callers must surface. `resource.version` supplied by the
    /// caller is advisory only and MUST NOT be trusted as the new value;
    /// this mirrors `WorkflowRepo`'s store-owned increment.
    async fn update(
        &self,
        resource: &ResourceEntry,
        expected_version: i64,
    ) -> Result<i64, StorageError>;

    /// Soft-delete a resource.
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError>;

    /// List **live** resources in a workspace with pagination.
    ///
    /// `offset`/`limit` page the workspace window (mirrors
    /// [`WorkflowRepo::list`](super::WorkflowRepo::list)). An
    /// implementation **MUST** exclude soft-deleted rows
    /// (`deleted_at IS NOT NULL`) **as part of** pagination — i.e. the
    /// `(offset, limit)` window is over the *live* row set, applied
    /// **after** the tombstone filter, never before. Paginating the raw
    /// window and filtering tombstones afterwards is forbidden: it yields
    /// sparse/short pages and can skip live rows entirely. A SQL backend
    /// therefore puts `WHERE deleted_at IS NULL` in the same query as
    /// `LIMIT`/`OFFSET`; an in-memory impl filters, then slices. With
    /// this contract the caller does no `deleted_at` post-filter.
    async fn list(
        &self,
        workspace_id: &[u8],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<ResourceEntry>, StorageError>;
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
