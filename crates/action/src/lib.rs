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
//! - [`ProcessAction`] — stateless single-execution action (most common)
//! - [`StatefulAction`] — iterative action with persistent state
//! - [`TriggerAction`] — event source that starts workflows
//! - [`ActionResult`] — execution result carrying data and flow-control intent
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
/// Capability declarations and isolation levels for sandboxed execution.
pub mod capability;
/// Runtime context provided to actions during execution.
pub mod context;
/// Error types distinguishing retryable from fatal failures.
pub mod error;
/// Static metadata, versioning, and execution mode descriptors.
pub mod metadata;
/// Output data representations (inline JSON and blob references).
pub mod output;
/// Execution result types carrying data and flow-control intent.
pub mod result;
/// Execution budget and data passing policies.
pub mod budget;
/// Action registry for type-erased discovery and lookup.
pub mod registry;
/// Sandboxed execution context and runner port trait.
pub mod sandbox;
mod types;

// ── Public re-exports ───────────────────────────────────────────────────────

pub use action::Action;
pub use capability::{Capability, IsolationLevel};
pub use context::ActionContext;
pub use error::ActionError;
pub use metadata::{ActionMetadata, ActionType, ExecutionMode, InterfaceVersion};
pub use output::NodeOutputData;
pub use result::{ActionResult, BreakReason, BranchKey, PortKey, WaitCondition};
pub use types::ProcessAction;
pub use types::StatefulAction;
pub use types::TriggerAction;
pub use types::trigger::{TriggerEvent, TriggerKind, WebhookRequest};

pub use budget::{DataPassingPolicy, ExecutionBudget, LargeDataStrategy};
pub use registry::ActionRegistry;
pub use sandbox::{SandboxRunner, SandboxedContext};
