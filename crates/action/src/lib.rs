//! # Nebula Action System
//!
//! Execution abstraction layer for Nebula workflow nodes.
//!
//! This crate defines **what** actions are and **how they communicate** with
//! the engine, but not how the engine orchestrates them. It follows the
//! Ports & Drivers architecture: core types live here, concrete execution
//! environments (in-process, WASM sandbox) are implemented as drivers.
//!
//! ## Core Types
//!
//! - [`Action`] — base trait providing identity and metadata
//! - [`SimpleAction`] — zero-boilerplate action returning `Result<Output, Error>`
//! - [`ProcessAction`] — stateless single-execution action with flow-control
//! - [`StatefulAction`] — iterative action with persistent state
//! - [`TriggerAction`] — event source that starts workflows
//! - [`StreamingAction`] — continuous stream producer
//! - [`TransactionalAction`] — distributed transaction participant (saga)
//! - [`InteractiveAction`] — human-in-the-loop interaction
//! - [`ActionResult`] — execution result carrying data and flow-control intent
//! - [`ActionOutput`] — first-class output type (value, binary, reference, stream)
//! - [`ActionError`] — error type distinguishing retryable from fatal failures
//! - [`ActionContext`] — runtime context with IDs, variables, cancellation
//! - [`ActionMetadata`] — static descriptor (key, version, capabilities)
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use nebula_action::*;
//! use async_trait::async_trait;
//!
//! struct MyAction { meta: ActionMetadata }
//!
//! impl Action for MyAction {
//!     fn metadata(&self) -> &ActionMetadata { &self.meta }
//!     fn action_type(&self) -> ActionType { ActionType::Process }
//! }
//!
//! #[async_trait]
//! impl ProcessAction for MyAction {
//!     type Input = serde_json::Value;
//!     type Output = serde_json::Value;
//!
//!     async fn execute(
//!         &self,
//!         input: Self::Input,
//!         ctx: &ActionContext,
//!     ) -> Result<ActionResult<Self::Output>, ActionError> {
//!         ctx.check_cancelled()?;
//!         Ok(ActionResult::success(input))
//!     }
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Base action trait defining identity and metadata.
pub mod action;
/// Action component collection for dependency declarations.
pub mod components;
/// Runtime context provided to actions during execution.
pub mod context;
/// Error types distinguishing retryable from fatal failures.
pub mod error;
/// Static metadata, versioning, and execution mode descriptors.
pub mod metadata;
/// Output data representations (inline JSON and blob references).
pub mod output;
/// Port definitions describing action input/output connection points.
pub mod port;
/// Convenience re-exports for action authors.
pub mod prelude;
/// Dependency-injection port traits (credentials, logging, metrics).
pub mod provider;
/// Action registry for type-erased discovery and lookup.
pub mod registry;
/// Execution result types carrying data and flow-control intent.
pub mod result;

// ── Public re-exports ───────────────────────────────────────────────────────

pub use action::Action;
pub use components::ActionComponents;
pub use context::ActionContext;
pub use error::ActionError;
pub use metadata::{
    ActionMetadata, ActionType, ExecutionMode, InterfaceVersion, RetryPolicy, TimeoutPolicy,
};
pub use output::{
    ActionOutput, BinaryData, BinaryStorage, BufferConfig, CacheInfo, Cost, DataReference,
    DeferredOutput, DeltaFormat, ExpectedOutput, OutputEnvelope, OutputMeta, OutputOrigin,
    Overflow, PollTarget, Producer, ProducerKind, Progress, Resolution, StreamMode, StreamOutput,
    StreamState, Timing, TokenUsage,
};
pub use port::{ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort};
pub use provider::{CredentialProvider, ResourceProvider, SecureString};
pub use result::{ActionResult, BranchKey, BreakReason, PortKey, WaitCondition};

pub use registry::ActionRegistry;

// Re-export parameter types so action authors can define parameters without
// depending on `nebula-parameter` directly.
pub use nebula_parameter::collection::ParameterCollection;
pub use nebula_parameter::def::ParameterDef;
