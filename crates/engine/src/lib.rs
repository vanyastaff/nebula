#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-engine — Composition Root
//!
//! Workflow execution orchestrator. Builds an `ExecutionPlan` from a workflow
//! DAG, resolves node inputs from predecessor outputs, transitions execution
//! state through `ExecutionRepo` (CAS on `version` — canon §11.1), and
//! delegates action dispatch to `nebula-runtime`.
//!
//! This crate is the **single real consumer** of `execution_control_queue`
//! in production deployment modes (canon §12.2). A handler that only logs
//! and discards control-queue rows does not satisfy the canon.
//!
//! ## Key types
//!
//! - `WorkflowEngine` — entry point; level-by-level DAG execution with bounded concurrency.
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
//! - §12.2 durable control plane; engine is the `execution_control_queue` consumer.
//!
//! See `crates/engine/README.md` for known open debts (budget ephemerality,
//! fail-open credential allowlist, edge-gate narrowness).

pub mod credential_accessor;
pub mod engine;
pub mod error;
pub mod event;
pub mod node_output;
pub(crate) mod resolver;
// pub(crate) mod resource;
pub mod resource_accessor;
pub mod result;

pub use credential_accessor::EngineCredentialAccessor;
pub use engine::{DEFAULT_EVENT_CHANNEL_CAPACITY, WorkflowEngine};
pub use error::EngineError;
pub use event::ExecutionEvent;
// Re-export plugin types for convenience.
pub use nebula_plugin::{Plugin, PluginKey, PluginMetadata, PluginRegistry, PluginType};
pub use node_output::NodeOutput;
pub use resource_accessor::EngineResourceAccessor;
pub use result::ExecutionResult;
