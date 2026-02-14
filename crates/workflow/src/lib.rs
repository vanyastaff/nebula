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
