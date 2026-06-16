//! Capability-routed job-dispatch pull loop for the Nebula orchestration layer.
//!
//! `nebula-orchestrator` owns exactly one thing: the **leaderless routing pull-loop** —
//! claim [`JobDispatchQueue`] rows whose `required_plugins ⊆ available_plugins`, hand each
//! [`JobDispatchMsg`] to an [`ExecutionSink`], fence-mark the row dispatched/failed, plus a
//! periodic [`JobDispatchQueue::reclaim_stuck`] sweep.
//!
//! ## What this crate defers
//!
//! - **Execution** → future `nebula-worker`: the real [`ExecutionSink`] drives the engine Start
//!   path, reads `PluginRegistry` at boot to derive `available_plugins`, and constructs an
//!   [`Orchestrator`] with a concrete sink implementation.
//! - **Enqueue** → `DurableExecutionEmitter`: the orchestrator never enqueues, only consumes.
//!
//! ## Dependency boundary (ADR-0095)
//!
//! Normal deps: `nebula-storage-port`, `nebula-core`, `nebula-metrics`.
//! No `nebula-engine`, `nebula-plugin`, or `nebula-storage` in `[dependencies]`.
//! `nebula-storage` appears only in `[dev-dependencies]` (the `InMemoryJobDispatchQueue`
//! used in integration tests).
//!
//! [`JobDispatchQueue`]: nebula_storage_port::store::JobDispatchQueue
//! [`JobDispatchQueue::reclaim_stuck`]: nebula_storage_port::store::JobDispatchQueue::reclaim_stuck
//! [`JobDispatchMsg`]: nebula_storage_port::dto::JobDispatchMsg

pub mod orchestrator;
pub mod sink;

pub use orchestrator::Orchestrator;
pub use sink::{ExecutionSink, ExecutionSinkError};
