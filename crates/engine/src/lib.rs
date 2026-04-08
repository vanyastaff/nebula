#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Engine
//!
//! Workflow execution orchestrator for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`WorkflowEngine`] — executes workflows level-by-level with bounded concurrency
//! - [`ExecutionResult`] — final result of a workflow execution
//! - [`EngineError`] — error types for the engine layer
//!
//! The engine sits between the user-facing API and the runtime. It builds
//! an execution plan from the workflow graph, resolves node inputs from
//! predecessor outputs, and delegates action execution to the runtime.

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
pub use engine::WorkflowEngine;
pub use error::EngineError;
pub use event::ExecutionEvent;
pub use node_output::NodeOutput;
pub use resource_accessor::EngineResourceAccessor;
pub use result::ExecutionResult;

// Re-export plugin types for convenience.
pub use nebula_plugin::{Plugin, PluginKey, PluginMetadata, PluginRegistry, PluginType};
