#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-workflow
//!
//! Workflow definition, DAG graph, and activation-time validator for the Nebula engine.
//!
//! **Role:** Workflow Definition + DAG + Validation. See `crates/workflow/README.md`.
//!
//! **Canon:** §10 (golden path — activation runs `validate_workflow`), §12.2 (shift-left
//! validation contract).
//!
//! **Maturity:** `stable` — definition types, builder, DAG, and validator are in active use.
//!
//! ## Core Types
//!
//! - [`WorkflowDefinition`] — top-level workflow; carries nodes, connections, config, UI metadata.
//! - [`NodeDefinition`] and [`ParamValue`] — individual steps and typed parameter values.
//! - [`Connection`] and [`EdgeCondition`] — directed edges with conditional routing.
//! - [`DependencyGraph`] — `petgraph` wrapper; topological sort + per-level batching.
//! - [`WorkflowBuilder`] — fluent, validated construction API.
//! - [`validate_workflow`] — multi-error validator; **canon §10 requires this at activation**.
//! - [`NodeState`] — execution progress tracking per node.
//!
//! ## Non-goals
//!
//! Not the execution state machine (`nebula-execution`), not the storage layer
//! (`nebula-storage` + `nebula-api`), not an expression evaluator (`nebula-expression`).

pub mod builder;
pub mod connection;
pub mod definition;
pub mod error;
pub mod graph;
pub mod node;
pub mod state;
pub mod validate;
pub mod version;

pub use builder::WorkflowBuilder;
pub use connection::{Connection, EdgeCondition, ErrorMatcher, ResultMatcher};
pub use definition::{
    Annotation, CURRENT_SCHEMA_VERSION, CheckpointingConfig, ErrorStrategy, NodePosition,
    RetryConfig, TriggerDefinition, UiMetadata, Viewport, WorkflowConfig, WorkflowDefinition,
};
pub use error::WorkflowError;
pub use graph::DependencyGraph;
/// Re-export the shared serde helper so internal `crate::serde_duration_opt` still resolves.
pub(crate) use nebula_core::serde_helpers::duration_opt_ms as serde_duration_opt;
pub use node::{NodeDefinition, ParamValue, RateLimit};
pub use state::NodeState;
pub use validate::validate_workflow;
pub use version::Version;
