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
//! - **implemented** — M3.5: [`control_consumer::ControlConsumer`] restores W3C trace parents from
//!   queue rows onto the per-dispatch span (`control_trace` + `tracing_opentelemetry`).
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
//! - `LayeredResourceAccessor` / `ScopedResourceMap` — Phase 6 (M6.1) precedence wiring. `scoped →
//!   global` lookup; closest-ancestor wins.
//! - `DashScopedResourceMap` / `BranchId` / `ScopedResourceGuard` — Phase 7 (M6.2) per-branch
//!   storage, RAII cleanup, and inner-to-outer + LIFO destroy ordering with 30s timeout per
//!   resource. Engine wiring of `ResourceAction::configure`/`cleanup` per branch is deferred (see
//!   `.ai-factory/PHASE7_BLOCKED.md`); the API surface is in place.
//!
//! ## Metrics registry wiring
//!
//! [`WorkflowEngine::new`] and [`runtime::ActionRuntime::try_new`] return [`Result`] if the shared
//! `MetricsRegistry` rejects registration for a canonical metric identity (same name bound to
//! incompatible primitive kinds, histogram bucket-layout conflict, etc.). Composition roots
//! should treat that like any other startup failure: log and abort or surface
//! [`error::EngineError::Telemetry`] / `nebula_metrics::MetricsError` to the caller.
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
mod control_trace;
pub mod credential;
pub mod credential_accessor;
pub mod daemon;
pub mod engine;
pub mod error;
pub mod event;
pub mod node_output;
pub(crate) mod resolver;
pub mod resource;
pub mod resource_accessor;
pub mod resource_status;
pub mod result;
pub mod runtime;
pub mod scoped_resources;

// Re-export the absorbed `nebula-engine` public surface at the crate root so
// every downstream caller can migrate `use crate::runtime::X` → `use
// nebula_engine::X` without path adjustments deeper than the crate name.
pub use control_consumer::{
    ControlConsumer, ControlDispatch, ControlDispatchError, DEFAULT_BATCH_SIZE,
    DEFAULT_POLL_INTERVAL, MAX_CLAIM_ERROR_BACKOFF,
};
pub use control_dispatch::EngineControlDispatch;
pub use credential::{
    CredentialResolver, ExecutorError, ResolveError, ResolveResponse, StateProjectionError,
    StateProjectionRegistry, execute_continue, execute_resolve,
};
pub use credential_accessor::EngineCredentialAccessor;
pub use daemon::{
    AnyDaemonHandle, Daemon, DaemonConfig, DaemonError, DaemonRegistry, DaemonRuntime, EventSource,
    EventSourceAdapter, EventSourceConfig, EventSourceRuntime, RestartPolicy,
};
pub use engine::{DEFAULT_EVENT_CHANNEL_CAPACITY, WorkflowEngine};
pub use error::EngineError;
pub use event::ExecutionEvent;
// Re-export plugin types for convenience.
pub use nebula_plugin::{Plugin, PluginKey, PluginManifest, PluginRegistry, ResolvedPlugin};
pub use node_output::NodeOutput;
pub use resource::{
    ErasedResourceRegistrar, RegisterRequest, RegistrarError, ResourceRegistrarRegistry,
    TypedResourceRegistrar,
};
pub use resource_accessor::EngineResourceAccessor;
pub use resource_status::{
    EngineManagerResourceStatus, EngineResourceStatus, ResourceRuntimeStatus,
};
pub use result::ExecutionResult;
pub use runtime::{
    ActionExecutor, ActionRegistry, ActionRuntime, BlobRef, BlobStorage, BoundedStreamBuffer,
    DataPassingPolicy, InProcessSandbox, LargeDataStrategy, MemoryQueue, PushOutcome, QueueError,
    RuntimeError, SandboxRunner, SandboxedContext, StatefulCheckpoint, StatefulCheckpointSink,
    TaskQueue,
};
pub use scoped_resources::{
    BranchId, CleanupOutcome, DEFAULT_CLEANUP_TIMEOUT, DashScopedResourceMap,
    EmptyScopedResourceMap, LayeredResourceAccessor, MAX_ANCESTOR_DEPTH, PoppedEntry, ScopedLookup,
    ScopedResourceGuard, ScopedResourceMap, run_cleanup, run_cleanup_with_timeout,
};
