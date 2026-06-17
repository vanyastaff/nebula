//! The engine's bundled view of the segregated storage port.
//!
//! The spec-16 storage port splits execution state CAS, the journal,
//! leases, node outputs/results, idempotency, and stateful checkpoints
//! into ISP-segregated object-safe traits. The engine legitimately needs
//! several of them together for one execution, so it holds them as a
//! single bundle — [`ExecutionStores`]. Every field is a port trait the
//! engine consumes directly; the bundle only spares every call site from
//! threading five `Arc`s.
//!
//! ## Tenancy
//!
//! The engine is tenant-agnostic. Every port call takes a `&Scope`, which
//! is threaded in from the per-message scope on the production path
//! (control-queue / job-dispatch row). On the worker path there is no
//! tenancy decorator, so the scope the engine passes is the real tenant
//! scope from the message DTO — cross-tenant isolation invariant #7.
//!
//! Tests use raw in-memory adapters (no decorator) and call
//! [`test_scope`] so every engine call observes one coherent fake tenant.
//!
//! ## Fencing
//!
//! `acquire_lease` returns a `FencingToken`. The engine threads that token
//! into every `TransitionBatch` it commits, so a superseded holder is
//! rejected by the store even if its CAS version still matches — the
//! zombie-runner hole stays closed end-to-end.

use std::sync::Arc;

use nebula_storage_port::dto::NodeResultRecord;
use nebula_storage_port::store::{
    CheckpointStore, ExecutionJournalReader, ExecutionStore, IdempotencyGuard, NodeResultStore,
    WorkflowStore, WorkflowVersionStore,
};
use nebula_storage_port::{FencingToken, Scope, StorageError};

/// Wrap a raw node-output payload in the port's [`NodeResultRecord`].
///
/// Raw outputs carry no `ActionResult` variant, so the kind tag is the
/// fixed `"Output"` marker; the schema version is the engine's current
/// node-result schema.
#[must_use]
pub fn node_output_record(json: serde_json::Value) -> NodeResultRecord {
    NodeResultRecord {
        kind_tag: "Output".to_owned(),
        json,
        schema_version: nebula_storage_port::dto::MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
    }
}

/// Wrap a serialized `ActionResult<Value>` in the port's
/// [`NodeResultRecord`], stamping the variant discriminant as the kind tag
/// so idempotent replay can reconstruct exact routing semantics.
pub fn node_result_record(json: serde_json::Value) -> Result<NodeResultRecord, StoreSeamError> {
    let Some(kind_tag) = json.get("type").and_then(serde_json::Value::as_str) else {
        return Err(StoreSeamError::MissingActionResultDiscriminant);
    };
    Ok(NodeResultRecord {
        kind_tag: kind_tag.to_owned(),
        json,
        schema_version: nebula_storage_port::dto::MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
    })
}

/// Errors raised while translating engine values into storage-port DTOs.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreSeamError {
    /// Serialized `ActionResult<T>` values must carry their serde tag.
    #[error("serialized ActionResult JSON missing `type` discriminant")]
    MissingActionResultDiscriminant,
}

/// Wrap the workflow trigger input in a [`NodeResultRecord`] for the
/// resume seam (`set_workflow_input` / `get_workflow_input`).
#[must_use]
pub fn workflow_input_record(json: serde_json::Value) -> NodeResultRecord {
    NodeResultRecord {
        kind_tag: "WorkflowInput".to_owned(),
        json,
        schema_version: nebula_storage_port::dto::MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
    }
}

/// Fixed fake scope used by tests, replay paths, and `execute_workflow`.
///
/// Production code threads the per-message scope (from the control-queue
/// or job-dispatch row) directly into every engine port call via
/// `resume_execution(scope, ...)`. This helper is used only where a real
/// per-message scope is unavailable:
///
/// - `execute_workflow` / `execute_workflow_with_acquire_scope` — test
///   and local library mode entry points (no control-queue message).
/// - `replay_execution` — mints a fresh `ExecutionId` and is lease-less;
///   never crosses a tenant boundary.
/// - All test wiring that bypasses the control queue.
///
/// The value is `pub` because integration tests (compiled as separate crates)
/// call it via `nebula_engine::store_seam::test_scope()`. `nebula-engine` is a
/// private impl-detail crate (not re-exported by `nebula-sdk`), so this is not
/// a public semver surface — it is test wiring internal to the engine workspace
/// member.
#[must_use]
pub fn test_scope() -> Scope {
    Scope::new("nebula", "nebula")
}

/// The engine's bundle of the storage-port traits it consumes for execution
/// state, leases, the journal, node results, idempotency, and checkpoints.
///
/// Constructed at the composition root from already-scoped (decorated)
/// handles. Every field is an independent port trait; the engine calls them
/// directly.
#[derive(Clone)]
pub struct ExecutionStores {
    /// State CAS + lease lifecycle + the atomic transition batch.
    pub execution: Arc<dyn ExecutionStore>,
    /// Append-only journal read side (appends go through the batch).
    pub journal: Arc<dyn ExecutionJournalReader>,
    /// Per-node output + typed result persistence.
    pub node_results: Arc<dyn NodeResultStore>,
    /// Best-effort stateful-action checkpoint persistence.
    pub checkpoints: Arc<dyn CheckpointStore>,
    /// Per-attempt idempotency guard.
    pub idempotency: Arc<dyn IdempotencyGuard>,
}

impl std::fmt::Debug for ExecutionStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionStores").finish_non_exhaustive()
    }
}

/// The engine's bundle of the workflow-definition port traits used by the
/// resume path to reload a persisted workflow.
#[derive(Clone)]
pub struct WorkflowStores {
    /// Workflow-row aggregate (slug / soft-delete / CAS version).
    pub workflow: Arc<dyn WorkflowStore>,
    /// Workflow-version aggregate (carries the opaque definition payload).
    pub versions: Arc<dyn WorkflowVersionStore>,
}

impl std::fmt::Debug for WorkflowStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowStores").finish_non_exhaustive()
    }
}

/// Typed lease-backend failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub(crate) enum LeaseBackendError {
    /// The storage port rejected or failed the lease operation.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// The engine's held execution lease, backed by the spec-16 port.
///
/// A [`FencingToken`] is threaded into every committed transition batch
/// so a superseded holder is rejected even on a matching CAS version
/// (the zombie-runner closure stays sound end-to-end).
#[derive(Clone)]
pub(crate) struct LeaseBackend {
    /// The scoped execution store used for renew / release.
    store: Arc<dyn ExecutionStore>,
    /// Bound scope (the tenancy decorator substitutes its own).
    scope: Scope,
    /// Fencing generation proving this runner currently owns the lease;
    /// threaded into every committed transition batch.
    token: FencingToken,
}

impl LeaseBackend {
    /// Build a port-backed lease handle.
    #[must_use]
    pub(crate) fn new(store: Arc<dyn ExecutionStore>, scope: Scope, token: FencingToken) -> Self {
        Self {
            store,
            scope,
            token,
        }
    }

    /// The fencing token gating every committed batch for this lease.
    /// Returned as `Option` so call sites that thread it through an
    /// already-optional lease (`Option<LeaseGuard>`) stay uniform.
    #[must_use]
    pub(crate) fn fencing_token(&self) -> Option<FencingToken> {
        Some(self.token)
    }

    /// Renew the lease. Returns `Ok(true)` when still held, `Ok(false)`
    /// when superseded/expired (the caller treats either non-true as
    /// loss).
    pub(crate) async fn renew(
        &self,
        execution_id: nebula_core::id::ExecutionId,
        ttl: std::time::Duration,
    ) -> Result<bool, LeaseBackendError> {
        Ok(self
            .store
            .renew_lease(&self.scope, &execution_id.to_string(), self.token, ttl)
            .await?)
    }

    /// Release the lease (best-effort). Returns `Ok(true)` when released,
    /// `Ok(false)` when it was no longer owned.
    pub(crate) async fn release(
        &self,
        execution_id: nebula_core::id::ExecutionId,
    ) -> Result<bool, LeaseBackendError> {
        Ok(self
            .store
            .release_lease(&self.scope, &execution_id.to_string(), self.token)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn node_result_record_uses_action_result_type_tag() {
        let record = node_result_record(json!({
            "type": "Success",
            "value": 42
        }))
        .expect("record");

        assert_eq!(record.kind_tag, "Success");
    }

    #[test]
    fn node_result_record_rejects_missing_type_tag() {
        let err = node_result_record(json!({
            "value": 42
        }))
        .expect_err("missing type must be rejected");

        assert!(matches!(
            err,
            StoreSeamError::MissingActionResultDiscriminant
        ));
    }
}
