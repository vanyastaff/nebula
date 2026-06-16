//! Execution row DTO.
use crate::Scope;
use serde::{Deserialize, Serialize};

/// Parameters for inserting a new execution row inside a compose transaction.
///
/// The execution id is taken from `JobDispatchMsg::execution_id` and the
/// tenant scope from `JobDispatchMsg::scope` — single source of truth in the
/// compose method; this struct carries only the fields that differ.
///
/// Construct via [`NewExecution::new`]; struct literal syntax is
/// unavailable from external crates (`#[non_exhaustive]`).
#[non_exhaustive]
pub struct NewExecution<'a> {
    /// Owning workflow id (opaque string form).
    pub workflow_id: &'a str,
    /// Initial execution state blob.
    pub initial_state: &'a serde_json::Value,
}

impl<'a> NewExecution<'a> {
    /// Construct a new-execution parameter set.
    pub fn new(workflow_id: &'a str, initial_state: &'a serde_json::Value) -> Self {
        Self {
            workflow_id,
            initial_state,
        }
    }
}

/// One execution row as the port exposes it.
///
/// `state` is opaque `serde_json::Value` by design: the port never
/// interprets execution state — the execution FSM lives in
/// `nebula-execution`. `fencing` is the lease generation that last wrote the
/// row (`None` before any lease is acquired).
// guard-justified: `state` is `serde_json::Value`, which is not `Eq`
// (it can hold a float). `Eq` is therefore not derivable; the clippy
// hint is a false positive for any DTO carrying an opaque JSON payload.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionRecord {
    /// Execution id (opaque string form).
    pub id: String,
    /// Owning workflow id (opaque string form).
    pub workflow_id: String,
    /// Tenant scope this row belongs to.
    pub scope: Scope,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Execution status (opaque to the port).
    pub status: String,
    /// Opaque execution state blob.
    pub state: serde_json::Value,
    /// Replica currently holding the lease, if any.
    pub lease_holder: Option<String>,
    /// Lease fencing generation that last wrote the row, if any.
    pub fencing: Option<u64>,
    /// Creation timestamp (RFC 3339).
    pub created_at: String,
    /// Last-update timestamp (RFC 3339).
    pub updated_at: String,
}
