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

pub mod engine;
pub mod error;
pub mod result;

pub use engine::WorkflowEngine;
pub use error::EngineError;
pub use result::ExecutionResult;

// Re-export node types for convenience.
pub use nebula_node::{Node, NodeKey, NodeMetadata, NodeRegistry, NodeType};
