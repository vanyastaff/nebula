#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-execution
//!
//! Execution state machine, journal, idempotency, and planning types for the Nebula engine.
//! Models execution-time concepts; does NOT contain the orchestrator or the storage implementation.
//!
//! **Role:** Execution State Machine + Journal + Idempotency Types.
//! See `crates/execution/README.md`.
//!
//! **Canon:** §11.1 (execution authority), §11.2 (retry debt), §11.3 (idempotency),
//! §11.5 (persistence matrix), §12.2 (single lifecycle).
//!
//! **Maturity:** `stable` — state machine, journal, and plan types in active use.
//! Engine-level retry accounting (`NodeAttempt` → re-execution) is `planned` (§11.2).
//!
//! ## Core Types
//!
//! - [`ExecutionStatus`] — 8-state machine; transitions validated by [`transition`] module.
//! - [`ExecutionState`], [`NodeExecutionState`] — persistent state tracking.
//! - [`ExecutionPlan`] — pre-computed parallel schedule derived from the workflow DAG.
//! - [`ExecutionContext`] — lightweight runtime context (`execution_id`, [`ExecutionBudget`]).
//! - [`ExecutionResult`] — post-execution summary.
//! - [`JournalEntry`] — audit log entry; backs `execution_journal` append-only table.
//! - [`NodeOutput`], [`ExecutionOutput`] — node output data with metadata.
//! - [`NodeAttempt`] — attempt tracking; engine re-execution is `planned`, not `implemented`.
//! - [`IdempotencyKey`] — deterministic key `{execution_id}:{node_id}:{attempt}`; dedup enforcement
//!   lives in `nebula_storage::ExecutionRepo`.
//! - [`ExecutionError`] — typed error for state machine violations.
//!
//! ## Non-goals
//!
//! Not the orchestrator (`nebula-engine`), not the storage implementation (`nebula-storage`),
//! not a retry scheduler (`nebula-resilience` inside an action is the canonical retry surface).

pub mod attempt;
pub mod context;
pub mod error;
pub mod idempotency;
pub mod journal;
pub mod output;
pub mod plan;
pub mod replay;
pub mod result;
pub mod state;
pub mod status;
pub mod transition;

pub use attempt::NodeAttempt;
pub use context::{ExecutionBudget, ExecutionContext};
pub use error::ExecutionError;
pub use idempotency::IdempotencyKey;
#[allow(deprecated)]
pub use idempotency::IdempotencyManager;
pub use journal::JournalEntry;
/// Re-export the shared serde helper so internal `crate::serde_duration_opt` still resolves.
pub(crate) use nebula_core::serde_helpers::duration_opt_ms as serde_duration_opt;
pub use output::{ExecutionOutput, NodeOutput};
pub use plan::ExecutionPlan;
pub use replay::ReplayPlan;
pub use result::ExecutionResult;
pub use state::{ExecutionState, NodeExecutionState};
pub use status::ExecutionStatus;
