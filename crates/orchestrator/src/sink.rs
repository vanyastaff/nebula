//! [`ExecutionSink`] — the orchestrator's DIP seam for execution hand-off.
//!
//! The orchestrator ends each dispatch claim with a durable handoff
//! ([`ExecutionTurnHandoff::accept_turn`]): the queue row is acknowledged and
//! the execution lease is acquired in one transaction. Only an accepted turn
//! reaches the sink. On `Ok` the orchestrator records `dispatched`; on `Err`
//! it records `failed` — but touches no queue state, because the handoff
//! already terminalised the row. What happens to the execution next is
//! governed by the execution lease and persisted recovery state, not by the
//! dispatch queue. The distinction between [`ExecutionSinkError::Rejected`]
//! and [`ExecutionSinkError::Internal`] is for operator dashboards, not retry
//! policy.
//!
//! Mirror of `ControlDispatchError {Rejected, Internal}` in `nebula-engine`'s
//! `control_consumer.rs`.
//!
//! [`ExecutionTurnHandoff::accept_turn`]: nebula_storage_port::store::ExecutionTurnHandoff::accept_turn

use nebula_storage_port::FencingToken;
use nebula_storage_port::dto::JobDispatchMsg;

/// One accepted execution turn handed to the sink.
///
/// Bundles the claimed [`JobDispatchMsg`] with the [`FencingToken`] the
/// durable handoff returned. By the time a sink sees a turn, the queue row is
/// already acknowledged and the execution lease is held under this fence —
/// the sink drives the turn **under** it rather than acquiring a lease of its
/// own. A second acquire would be rejected outright: a live lease blocks
/// acquisition even for the same holder, so the handoff's fence is the only
/// authority this turn runs under.
#[derive(Debug, Clone, Copy)]
pub struct DispatchedTurn<'a> {
    /// The claimed job-dispatch message.
    pub msg: &'a JobDispatchMsg,
    /// Fence proving ownership of the accepted turn. Every write the
    /// execution makes must be gated by it; once a reclaim supersedes it the
    /// turn's writes are rejected.
    pub fence: FencingToken,
}

/// Hand-off seam between the orchestrator pull-loop and execution.
///
/// The future `nebula-worker` crate provides the real implementation, which
/// drives the engine resume path under the turn's fence. Tests use a spy
/// (`RecordingSink` in `crates/orchestrator/tests/`).
///
/// ## Idempotency contract
///
/// After the handoff the queue row is terminal, so the job queue itself never
/// redelivers a dispatched turn. Implementations MUST still be idempotent per
/// `(execution_id, command)`: a second queue row for the same execution (e.g.
/// a restart fan-out) and the control-queue consumer both reach the same
/// engine state, and driving an execution that is already running or terminal
/// must return `Ok(())`, not an error.
///
/// ## Dyn-dispatch
///
/// `async-trait` is required because `async fn` in traits is not yet
/// dyn-compatible in stable Rust 1.97 (native AFIT/RPITIT is not dyn-safe).
/// The orchestrator holds an `Arc<dyn ExecutionSink>`, so object safety is
/// load-bearing here — the same rationale as `JobDispatchQueue` and
/// `ControlDispatch`.
#[async_trait::async_trait]
pub trait ExecutionSink: Send + Sync + std::fmt::Debug {
    /// Hand off an accepted turn to the execution layer.
    ///
    /// The queue row was already acknowledged by the handoff before this call;
    /// neither outcome touches queue state. `Ok` records `dispatched`; `Err`
    /// records `failed` and leaves the execution to lease-governed recovery.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionSinkError::Rejected`] when the execution layer
    /// performs a domain-level rejection (e.g. the execution is already
    /// terminal). Returns [`ExecutionSinkError::Internal`] on a transport or
    /// engine-internal failure. Both record `failed` at the orchestrator
    /// layer.
    async fn dispatch(&self, turn: &DispatchedTurn<'_>) -> Result<(), ExecutionSinkError>;
}

/// Errors returned from [`ExecutionSink::dispatch`].
///
/// Mirrors `ControlDispatchError` in the engine's `control_consumer` module.
/// Both variants record `failed` at the orchestrator layer; the split is for
/// operator dashboards (domain reject vs engine/transport failure).
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
