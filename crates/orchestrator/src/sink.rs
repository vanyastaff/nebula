//! [`ExecutionSink`] — the orchestrator's DIP seam for execution hand-off.
//!
//! The orchestrator calls [`ExecutionSink::dispatch`] for each claimed
//! [`JobDispatchMsg`]. On `Ok` it marks the row dispatched; on `Err` it marks it
//! failed.  The distinction between [`ExecutionSinkError::Rejected`] and
//! [`ExecutionSinkError::Internal`] is for operator dashboards, not retry policy:
//! both outcomes mark the row failed at this layer.
//!
//! Mirror of `ControlDispatchError {Rejected, Internal}` in `nebula-engine`'s
//! `control_consumer.rs`.
//!
//! [`JobDispatchMsg`]: nebula_storage_port::dto::JobDispatchMsg

use nebula_storage_port::dto::JobDispatchMsg;

/// Hand-off seam between the orchestrator pull-loop and execution.
///
/// The future `nebula-worker` crate provides the real implementation, which
/// drives the engine Start path. Tests use a spy (`RecordingSink` in
/// `crates/orchestrator/tests/`).
///
/// ## Idempotency contract
///
/// Implementations MUST be idempotent per `(execution_id, command)`: the
/// reclaim sweep can redeliver a job whose `dispatch` succeeded but whose
/// `mark_dispatched` failed (the row stays `Processing` until reclaimed).
/// Re-delivering to a sink that has already processed that pair must return
/// `Ok(())`, not an error.
///
/// ## Dyn-dispatch
///
/// `async-trait` is required because `async fn` in traits is not yet
/// dyn-compatible in stable Rust 1.96 (native AFIT/RPITIT is not dyn-safe).
/// The orchestrator holds an `Arc<dyn ExecutionSink>`, so object safety is
/// load-bearing here — the same rationale as `JobDispatchQueue` and
/// `ControlDispatch`.
#[async_trait::async_trait]
pub trait ExecutionSink: Send + Sync + std::fmt::Debug {
    /// Hand off a routed, claimed job to the execution layer.
    ///
    /// The orchestrator marks the row `Dispatched` on `Ok` and calls
    /// [`JobDispatchQueue::mark_failed`] on `Err`. Both error variants
    /// produce a `mark_failed` — the variant is for operator dashboards.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionSinkError::Rejected`] when the execution layer
    /// performs a domain-level rejection (e.g. the execution is already
    /// terminal). Returns [`ExecutionSinkError::Internal`] on a transport or
    /// engine-internal failure. Both produce `mark_failed` at the orchestrator
    /// layer.
    ///
    /// [`JobDispatchQueue::mark_failed`]: nebula_storage_port::store::JobDispatchQueue::mark_failed
    async fn dispatch(&self, msg: &JobDispatchMsg) -> Result<(), ExecutionSinkError>;
}

/// Errors returned from [`ExecutionSink::dispatch`].
///
/// Mirrors `ControlDispatchError` in the engine's `control_consumer` module.
/// Both variants result in `mark_failed` at the orchestrator layer; the split
/// is for operator dashboards (domain reject vs engine/transport failure).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecutionSinkError {
    /// The execution layer rejected the job (e.g. already terminal).
    ///
    /// Domain-level — not a bug; the operator dashboard can distinguish
    /// legitimate rejects from engine failures.
    #[error("execution sink rejected job: {0}")]
    Rejected(String),

    /// An engine or transport failure prevented dispatch.
    ///
    /// Distinct from [`Rejected`](Self::Rejected) so operators can identify
    /// engine bugs separately from expected domain rejects.
    #[error("execution sink failed: {0}")]
    Internal(String),
}
