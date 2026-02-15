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
//! - [`ExecutionContext`] — runtime context with shared state and cancellation
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
pub mod state;
pub mod status;
pub mod transition;

pub use attempt::NodeAttempt;
pub use context::ExecutionContext;
pub use error::ExecutionError;
pub use idempotency::{IdempotencyKey, IdempotencyManager};
pub use journal::JournalEntry;
pub use output::{ExecutionOutput, NodeOutput};
pub use plan::ExecutionPlan;
pub use state::{ExecutionState, NodeExecutionState};
pub use status::ExecutionStatus;

/// Serde helper for `Option<Duration>` serialized as milliseconds.
pub(crate) mod serde_duration_opt {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    /// Serialize an `Option<Duration>` as an optional integer of milliseconds.
    pub fn serialize<S: Serializer>(duration: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match duration {
            Some(d) => (d.as_millis() as u64).serialize(s),
            None => s.serialize_none(),
        }
    }

    /// Deserialize an optional integer of milliseconds into `Option<Duration>`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt: Option<u64> = Option::deserialize(d)?;
        Ok(opt.map(Duration::from_millis))
    }
}
