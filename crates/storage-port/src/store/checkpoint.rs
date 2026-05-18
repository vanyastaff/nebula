//! Stateful-checkpoint store trait (spec-16 §11.5).
use crate::error::StorageError;
use crate::scope::Scope;

/// Best-effort stateful-action checkpoint persistence.
///
/// **Best-effort semantics:** a checkpoint is a resumption optimisation, not
/// a durability guarantee. A missing checkpoint means "replay from the last
/// committed state", never data loss — the authoritative state is the
/// execution row written through [`crate::TransitionBatch`].
#[async_trait::async_trait]
pub trait CheckpointStore: Send + Sync + std::fmt::Debug {
    /// Save a stateful-action checkpoint (best-effort).
    async fn save_stateful_checkpoint(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
        checkpoint: serde_json::Value,
    ) -> Result<(), StorageError>;

    /// Load a stateful-action checkpoint, if one was persisted.
    async fn load_stateful_checkpoint(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<serde_json::Value>, StorageError>;
}
