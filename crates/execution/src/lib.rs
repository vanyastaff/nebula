#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! # nebula-execution
//!
//! Execution state machine, journal, idempotency, and planning types for the Nebula engine.
//! Models execution-time concepts; does NOT contain the orchestrator or the storage implementation.
//!
//! **Role:** Execution State Machine + Journal + Idempotency Types.
//! See `crates/execution/README.md`.
//!
//! **Maturity:** `stable` — state machine, journal, plan, and retry-state shapes in active use.
//! The engine owns operator-declared node retry; `nebula-resilience` remains the in-action
//! outbound-call retry surface.
//!
//! ## Core Types
//!
//! - [`ExecutionStatus`] — 8-state machine; transitions validated by [`transition`] module.
//! - [`ExecutionState`], [`NodeExecutionState`] — persistent state tracking.
//! - [`ExecutionPlan`] — pre-computed parallel schedule derived from the workflow DAG.
//! - [`ExecutionContext`] — lightweight runtime context (`execution_id`, [`ExecutionBudget`],
//!   optional [`W3cTraceContext`] for M3.5 trace propagation).
//! - [`ExecutionResult`] — post-execution summary.
//! - [`JournalEntry`] — audit log entry; backs `execution_journal` append-only table.
//! - [`NodeOutput`], [`ExecutionOutput`] — node output data with metadata.
//! - [`NodeAttempt`] — attempt-keyed shape used by `save_node_output`; operator-declared
//!   engine retry advances the attempt number on re-dispatch.
//! - [`IdempotencyKey`] — deterministic key `{execution_id}:{node_id}:{attempt}`; dedup enforcement
//!   lives behind the storage port's idempotency guard.
//! - `ExecutionRevisions` — experimental revision-pin vocabulary, available only with the
//!   explicitly unstable `unstable-revisions` feature.
//! - [`ExecutionError`] — typed error for state machine violations.
//!
//! ## Non-goals
//!
//! Not the orchestrator (`nebula-engine`), not the storage implementation (`nebula-storage`),
//! not a retry scheduler (the engine drives operator-declared retry; `nebula-resilience` covers
//! in-action outbound calls).

pub mod attempt;
pub mod context;
pub mod error;
pub mod idempotency;
pub mod journal;
pub mod output;
pub mod plan;
pub mod replay;
pub mod result;
#[cfg(feature = "unstable-revisions")]
pub mod revision;
pub mod state;
pub mod status;
pub mod transition;

pub use attempt::NodeAttempt;
pub use context::{ExecutionBudget, ExecutionContext};
pub use error::ExecutionError;
pub use idempotency::IdempotencyKey;
pub use journal::JournalEntry;
pub use nebula_core::W3cTraceContext;
/// Re-export the shared serde helper so internal `crate::serde_duration_opt` still resolves.
pub(crate) use nebula_core::serde_helpers::duration_opt_ms as serde_duration_opt;
pub use output::{ExecutionOutput, NodeOutput};
pub use plan::ExecutionPlan;
pub use replay::ReplayPlan;
pub use result::ExecutionResult;
#[cfg(feature = "unstable-revisions")]
pub use revision::ExecutionRevisions;
pub use state::{ExecutionState, NodeExecutionState};
pub use status::ExecutionStatus;
