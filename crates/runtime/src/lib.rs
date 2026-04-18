#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-runtime — Action Dispatcher
//!
//! Sits between `nebula-engine` (scheduler) and `nebula-sandbox` (isolation).
//! Resolves action handlers from the registry, enforces data-passing policies,
//! emits telemetry, and delegates to the sandbox for the actual call.
//!
//! ## Key types
//!
//! - `ActionRuntime` — executes a resolved action through the sandbox with data limits.
//! - `ActionRegistry` — registers and looks up action handlers by key.
//! - `DataPassingPolicy`, `LargeDataStrategy` — output size enforcement.
//! - `MemoryQueue`, `TaskQueue` — in-memory task queueing (not durable; durable control signals
//!   live in `execution_control_queue` — §12.2).
//! - `BlobRef`, `BlobStorage` — side-channel for large payloads.
//! - `StatefulCheckpoint`, `StatefulCheckpointSink` — checkpoint boundaries for `StatefulAction`
//!   types.
//! - `BoundedStreamBuffer`, `PushOutcome` — streaming with backpressure.
//! - `RuntimeError` — typed error.
//!
//! Re-exported from `nebula-sandbox` via `pub use`:
//! `ActionExecutor`, `InProcessSandbox`, `SandboxRunner`, `SandboxedContext`.
//!
//! ## Canon
//!
//! - §3.5 action dispatch by trait.
//! - §11.2 retry surface: dispatch only; engine-level re-execution is `planned`.
//! - §12.6 isolation honesty: isolation is by sandbox delegation.
//!
//! See `crates/runtime/README.md` for open debts.

pub mod blob;
pub mod data_policy;
pub mod error;
pub mod queue;
pub mod registry;
pub mod runtime;
pub mod stream_backpressure;

pub use blob::{BlobRef, BlobStorage};
pub use data_policy::{DataPassingPolicy, LargeDataStrategy};
pub use error::RuntimeError;
pub use nebula_sandbox::{ActionExecutor, InProcessSandbox, SandboxRunner, SandboxedContext};
pub use queue::{MemoryQueue, QueueError, TaskQueue};
pub use registry::ActionRegistry;
pub use runtime::{ActionRuntime, StatefulCheckpoint, StatefulCheckpointSink};
pub use stream_backpressure::{BoundedStreamBuffer, PushOutcome};
