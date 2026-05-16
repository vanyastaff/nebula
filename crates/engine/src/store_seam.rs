//! The engine's bundled view of the segregated storage port.
//!
//! The legacy `nebula_storage::ExecutionRepo` fused state CAS, the journal,
//! leases, node outputs/results, idempotency, and stateful checkpoints into
//! one god-trait. The spec-16 port splits these into ISP-segregated
//! object-safe traits. The engine legitimately needs several of them
//! together for one execution, so it holds them as a single bundle —
//! [`ExecutionStores`]. This is **not** a back-compat shim over the old
//! trait: every field is a port trait the engine consumes directly; the
//! bundle only spares every call site from threading five `Arc`s.
//!
//! ## Tenancy
//!
//! The engine is tenant-agnostic. Every port call takes a `&Scope`, but the
//! composition root wraps each store in `nebula_tenancy`'s scope-enforcing
//! decorator, which **substitutes** the bound tenant scope and ignores
//! whatever the engine passes. The engine therefore passes a single fixed
//! [`engine_scope`] placeholder and structurally cannot reach across
//! tenants (it never holds the raw adapter — only the decorated handle).
//! Test wiring that uses raw in-memory adapters (no decorator) is internally
//! consistent because every engine call uses the same placeholder scope.
//!
//! ## Fencing
//!
//! `acquire_lease` returns a `FencingToken`. The engine threads that token
//! into every `TransitionBatch` it commits, so a superseded holder is
//! rejected by the store even if its CAS version still matches — the
//! zombie-runner hole stays closed end-to-end.

use std::sync::Arc;

use nebula_storage_port::Scope;
use nebula_storage_port::dto::NodeResultRecord;
use nebula_storage_port::store::{
    CheckpointStore, ExecutionJournalReader, ExecutionStore, IdempotencyGuard, NodeResultStore,
    WorkflowStore, WorkflowVersionStore,
};

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
#[must_use]
pub fn node_result_record(json: serde_json::Value) -> NodeResultRecord {
    let kind_tag = json
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Unknown")
        .to_owned();
    NodeResultRecord {
        kind_tag,
        json,
        schema_version: nebula_storage_port::dto::MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
    }
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

/// The fixed placeholder scope every engine port call carries.
///
/// The tenancy decorator substitutes the request's bound scope and ignores
/// this value, so its concrete contents are irrelevant in production. In
/// decorator-less test wiring every call uses this same scope, so the raw
/// in-memory adapters behave as a single coherent tenant.
#[must_use]
pub fn engine_scope() -> Scope {
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
    /// Per-node output + typed result persistence (ADR-0009 resume seam).
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
