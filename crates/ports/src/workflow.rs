//! Workflow repository port.
//!
//! Defines the persistence interface for workflow definitions.
//! Backend drivers (SQLite, Postgres, in-memory) implement this trait.

use async_trait::async_trait;
use nebula_core::WorkflowId;

use crate::error::PortsError;

/// Persistence interface for workflow definitions.
///
/// All methods are async and object-safe. Implementations must be `Send + Sync`
/// so the trait object can be shared across Tokio tasks.
#[async_trait]
pub trait WorkflowRepo: Send + Sync {
    /// Get a workflow definition by ID. Returns serialized JSON.
    async fn get(&self, id: WorkflowId) -> Result<Option<serde_json::Value>, PortsError>;

    /// Save a workflow definition with optimistic concurrency.
    /// `version` is the expected current version for CAS.
    async fn save(
        &self,
        id: WorkflowId,
        version: u64,
        definition: serde_json::Value,
    ) -> Result<(), PortsError>;

    /// Delete a workflow by ID. Returns true if it existed.
    async fn delete(&self, id: WorkflowId) -> Result<bool, PortsError>;

    /// List workflow definitions with pagination.
    async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(WorkflowId, serde_json::Value)>, PortsError>;
}
