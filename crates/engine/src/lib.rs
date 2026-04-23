#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-engine — Composition Root
//!
//! Workflow execution orchestrator. Builds an `ExecutionPlan` from a workflow
//! DAG, resolves node inputs from predecessor outputs, transitions execution
//! state through `ExecutionRepo` (CAS on `version` — canon §11.1), and
//! delegates action dispatch to `nebula-engine`.
//!
//! Canon §12.2 names this crate as the location of the `execution_control_queue`
//! consumer (`ControlConsumer`, see [`control_consumer`]). Status per §11.6:
//!
//! - **implemented** — consumer skeleton: construction, polling loop with graceful shutdown,
//!   `claim_pending` / `mark_completed` / `mark_failed` plumbing, command observation with typed
//!   `ExecutionId` decoding.
//! - **implemented** — `Start` / `Resume` / `Restart` dispatch into the engine start / resume path
//!   (ADR-0008 follow-up A2; closes #332 / #327). The engine-owned implementation lives in
//!   [`control_dispatch::EngineControlDispatch`].
//! - **implemented** — `Cancel` / `Terminate` dispatch into the engine cancel path (ADR-0008
//!   follow-up A3; closes #330). `Cancel` signals the live frontier loop via
//!   [`WorkflowEngine::cancel_execution`]; `Terminate` shares the cooperative-cancel body until a
//!   distinct forced-shutdown path is wired (see ADR-0016).
//!
//! Wiring and atomicity decisions live in `docs/adr/0008-execution-control-queue-consumer.md`
//! and `docs/adr/0016-engine-cancel-registry.md`.
//!
//! ## Key types
//!
//! - `WorkflowEngine` — entry point; level-by-level DAG execution with bounded concurrency.
//! - `ControlConsumer` / `ControlDispatch` — durable control-queue consumer (§12.2, ADR-0008).
//! - `EngineControlDispatch` — canonical engine-side `ControlDispatch` impl (ADR-0008 A2).
//! - `ExecutionResult` — post-run summary returned to the API layer.
//! - `EngineError` — typed engine-layer error.
//! - `ExecutionEvent` — broadcast event type for `nebula-eventbus`.
//! - `EngineCredentialAccessor` / `EngineResourceAccessor` — scoped accessors injected into action
//!   contexts.
//!
//! ## Canon
//!
//! - §10 golden path (orchestrator schedules activated workflows).
//! - §11.1 execution authority via `ExecutionRepo`.
//! - §12.2 durable control plane; engine owns the `execution_control_queue` consumer.
//!
//! See `crates/engine/README.md` for known open debts (budget ephemerality,
//! edge-gate narrowness).

pub mod control_consumer;
pub mod control_dispatch;
pub mod credential;
pub mod credential_accessor;
pub mod engine;
pub mod error;
pub mod event;
pub mod node_output;
pub(crate) mod resolver;
// pub(crate) mod resource;
pub mod resource_accessor;
pub mod result;
pub mod runtime;

// Re-export the absorbed `nebula-engine` public surface at the crate root so
// every downstream caller can migrate `use crate::runtime::X` → `use
// nebula_engine::X` without path adjustments deeper than the crate name.
pub use control_consumer::{
    ControlConsumer, ControlDispatch, ControlDispatchError, DEFAULT_BATCH_SIZE,
    DEFAULT_POLL_INTERVAL, MAX_CLAIM_ERROR_BACKOFF,
};
pub use control_dispatch::EngineControlDispatch;
pub use credential::{
    CredentialRegistry, CredentialResolver, ExecutorError, RegistryError, ResolveError,
    ResolveResponse, execute_continue, execute_resolve,
};
pub use credential_accessor::EngineCredentialAccessor;
pub use engine::{DEFAULT_EVENT_CHANNEL_CAPACITY, WorkflowEngine};
pub use error::EngineError;
pub use event::ExecutionEvent;
// Re-export plugin types for convenience.
pub use nebula_plugin::{Plugin, PluginKey, PluginManifest, PluginRegistry, ResolvedPlugin};
pub use node_output::NodeOutput;
pub use resource_accessor::EngineResourceAccessor;
pub use result::ExecutionResult;
pub use runtime::{
    ActionExecutor, ActionRegistry, ActionRuntime, BlobRef, BlobStorage, BoundedStreamBuffer,
    DataPassingPolicy, InProcessSandbox, LargeDataStrategy, MemoryQueue, PushOutcome, QueueError,
    RuntimeError, SandboxRunner, SandboxedContext, StatefulCheckpoint, StatefulCheckpointSink,
    TaskQueue,
};
