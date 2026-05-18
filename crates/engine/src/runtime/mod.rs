//! Action Dispatcher (formerly the separate `nebula-engine` crate).
//!
//! Absorbed into `nebula-engine` per spec 28: the dispatcher and the
//! orchestrator share a lifecycle, a metrics surface, and a set of
//! context types — keeping them in two crates made every change a
//! two-step dance without any architectural benefit.
//!
//! ## Key types
//!
//! - [`ActionRuntime`] — executes a resolved action through the sandbox with data limits.
//! - [`ActionRegistry`] — registers and looks up action handlers by key.
//! - [`DataPassingPolicy`], [`LargeDataStrategy`] — output size enforcement.
//! - [`MemoryQueue`], [`TaskQueue`] — in-memory task queueing (not durable; durable control signals
//!   live in `execution_control_queue`).
//! - [`BlobRef`], [`BlobStorage`] — side-channel for large payloads.
//! - [`StatefulCheckpoint`], [`StatefulCheckpointSink`] — checkpoint boundaries for
//!   `StatefulAction` types.
//! - [`BoundedStreamBuffer`], [`PushOutcome`] — streaming with backpressure.
//! - [`RuntimeError`] — typed error surface.
//!
//! ## Canon
//!
//! - action dispatch by trait.
//! - retry surface: dispatch only; engine-level re-execution is `planned`.
//! - isolation honesty: isolation is by sandbox delegation.

pub mod blob;
pub mod data_policy;
pub mod error;
#[cfg(feature = "out-of-process-plugins")]
pub mod out_of_process;
pub(crate) mod plugin_pool;
#[cfg(feature = "out-of-process-plugins")]
pub mod plugin_supervisor;
pub mod queue;
pub mod registry;
#[allow(
    clippy::module_inception,
    reason = "runtime/runtime.rs carries ActionRuntime; kept stable for external callers"
)]
pub mod runtime;
pub mod sandbox_runner;
pub mod stream_backpressure;

pub use blob::{BlobRef, BlobStorage};
pub use data_policy::{DataPassingPolicy, LargeDataStrategy};
pub use error::RuntimeError;
#[cfg(feature = "out-of-process-plugins")]
pub use out_of_process::{OutOfProcessConfig, discover_into_registry};
#[cfg(feature = "out-of-process-plugins")]
pub use plugin_supervisor::PluginSupervisor;
pub use queue::{MemoryQueue, QueueError, TaskQueue};
pub use registry::ActionRegistry;
pub use runtime::{ActionRuntime, StatefulCheckpoint, StatefulCheckpointSink};
pub use sandbox_runner::{ActionExecutor, InProcessSandbox, SandboxRunner, SandboxedContext};
pub use stream_backpressure::{BoundedStreamBuffer, PushOutcome};
