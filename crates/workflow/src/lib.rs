#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Workflow
//!
//! Workflow definition, DAG graph, and validation for the Nebula workflow engine.
//!
//! This crate provides the types for defining workflows as directed acyclic graphs
//! (DAGs) of action nodes connected by conditional edges. It includes:
//!
//! - [`WorkflowDefinition`] and supporting config types
//! - [`NodeDefinition`] and [`ParamValue`] for individual steps
//! - [`Connection`] and [`EdgeCondition`] for edges between nodes
//! - [`DependencyGraph`] (a `petgraph` wrapper) for topological sorting and level computation
//! - [`WorkflowBuilder`] for fluent, validated construction
//! - [`validate_workflow`] for comprehensive multi-error validation
//! - [`NodeState`] for tracking execution progress

pub mod builder;
pub mod connection;
pub mod definition;
pub mod error;
pub mod graph;
pub mod node;
pub mod state;
pub mod validate;

pub use builder::WorkflowBuilder;
pub use connection::{Connection, EdgeCondition, ErrorMatcher, ResultMatcher};
pub use definition::{CheckpointingConfig, RetryConfig, WorkflowConfig, WorkflowDefinition};
pub use error::WorkflowError;
pub use graph::DependencyGraph;
pub use node::{NodeDefinition, ParamValue};
pub use state::NodeState;
pub use validate::validate_workflow;

/// Re-export the shared serde helper so internal `crate::serde_duration_opt` still resolves.
pub(crate) use nebula_core::serde_helpers::duration_opt_ms as serde_duration_opt;
