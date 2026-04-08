#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Execution
//!
//! Runtime execution state, journals, idempotency, and planning for the Nebula
//! workflow engine.
//!
//! This crate models execution-time concepts — it does NOT contain the engine
//! orchestrator. It defines:
//!
//! - [`ExecutionStatus`] — execution-level state machine (8 states)
//! - [`ExecutionState`] and [`NodeExecutionState`] — persistent state tracking
//! - [`ExecutionPlan`] — pre-computed parallel execution schedule
//! - [`ExecutionContext`] — lightweight runtime context (execution_id, budget)
//! - [`ExecutionResult`] — post-execution summary (status, timing, node counts, outputs)
//! - [`JournalEntry`] — audit log of execution events
//! - [`NodeOutput`] — node output data with metadata
//! - [`NodeAttempt`] — individual execution attempt tracking
//! - [`IdempotencyKey`] and [`IdempotencyManager`] — exactly-once guarantees
//! - State machine transitions validated by the [`transition`] module

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
pub use idempotency::{IdempotencyKey, IdempotencyManager};
pub use journal::JournalEntry;
pub use output::{ExecutionOutput, NodeOutput};
pub use plan::ExecutionPlan;
pub use replay::ReplayPlan;
pub use result::ExecutionResult;
pub use state::{ExecutionState, NodeExecutionState};
pub use status::ExecutionStatus;

/// Re-export the shared serde helper so internal `crate::serde_duration_opt` still resolves.
pub(crate) use nebula_core::serde_helpers::duration_opt_ms as serde_duration_opt;
