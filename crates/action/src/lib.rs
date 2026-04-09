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
//! - `SimpleAction` — zero-boilerplate action returning `Result<Output, Error>`
//! - [`StatelessAction`] — stateless single-execution action with flow-control
//! - [`StatefulAction`] — iterative action with persistent state (Continue/Break)
//! - [`TriggerAction`] — workflow starter (start/stop), outside execution graph
//! - [`ResourceAction`] — graph-level DI (configure/cleanup), scoped to downstream branch
//! - [`StatefulAction`] — iterative action with persistent state
//! - [`TriggerAction`] — event source that starts workflows
//! - `StreamingAction` — continuous stream producer
//! - `TransactionalAction` — distributed transaction participant (saga)
//! - `InteractiveAction` — human-in-the-loop interaction
//! - [`ActionResult`] — execution result carrying data and flow-control intent
//! - [`ActionOutput`] — first-class output type (value, binary, reference, stream)
//! - [`ActionError`] — error type distinguishing retryable from fatal failures
//! - [`Context`] — base trait for execution contexts
//! - [`ActionMetadata`] — static descriptor (key, version, capabilities)
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use nebula_action::*;
//!
//! struct MyAction { meta: ActionMetadata }
//!
//! impl ActionDependencies for MyAction {}
//!
//! impl Action for MyAction {
//!     fn metadata(&self) -> &ActionMetadata { &self.meta }
//! }
//!
//! impl StatelessAction for MyAction {
//!     type Input = serde_json::Value;
//!     type Output = serde_json::Value;
//!
//!     async fn execute(&self, input: Self::Input, _ctx: &impl Context)
//!         -> Result<ActionResult<Self::Output>, ActionError>
//!     {
//!         Ok(ActionResult::success(input))
//!     }
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Base action trait defining identity and metadata.
pub mod action;
/// Ergonomic authoring helpers for low-boilerplate actions.
pub mod authoring;
/// Capability interfaces injected into contexts (resources, credentials, logger).
pub mod capability;
/// Runtime context provided to actions during execution.
pub mod context;
/// Declarative dependency declaration for actions.
pub mod dependency;
/// Error types distinguishing retryable from fatal failures.
pub mod error;
/// Execution sub-traits (StatelessAction, etc.).
pub mod execution;
/// Extension traits for ergonomic error conversion in actions.
pub mod ext;
/// Credential guard — re-exported from [`nebula_credential`].
pub mod guard {
    pub use nebula_credential::CredentialGuard;
}
/// Dynamic handler contract for runtime (registry key → execute).
pub mod handler;
/// Assertion macros for testing action results (`assert_success!`, etc.).
mod macros;
/// Static metadata, versioning, and execution mode descriptors.
pub mod metadata;
/// Output data representations (inline JSON and blob references).
pub mod output;
/// Port definitions describing action input/output connection points.
pub mod port;
/// Convenience re-exports for action authors.
pub mod prelude;
/// Type-safe registry for action handlers, keyed by ActionKey.
pub mod registry;
/// Execution result types carrying data and flow-control intent.
pub mod result;
/// Scoped credential accessor — enforces type-based access control.
pub mod scoped;
/// Test utilities for action authors.
pub mod testing;
/// Action package validation utilities.
pub mod validation;

// ── Public re-exports ───────────────────────────────────────────────────────

pub use nebula_action_macros::Action;

pub use action::Action;
pub use authoring::{FnStatelessAction, FnStatelessCtxAction, stateless_ctx_fn, stateless_fn};
pub use capability::{
    ActionLogLevel, ActionLogger, CredentialAccessError, CredentialAccessor, ExecutionEmitter,
    NoopCredentialAccessor, ResourceAccessor, TriggerScheduler,
};
pub use context::{ActionContext, Context, TriggerContext};
pub use dependency::ActionDependencies;
pub use error::{ActionError, ErrorCode};
pub use execution::{ResourceAction, StatefulAction, StatelessAction, TriggerAction};
pub use ext::ActionResultExt;
#[allow(deprecated)]
// Reason: InternalHandler re-exported for backward compat during migration
pub use handler::InternalHandler;
pub use handler::{
    ActionHandler, AgentHandler, ResourceActionAdapter, ResourceHandler, StatefulActionAdapter,
    StatefulHandler, StatelessActionAdapter, StatelessHandler, TriggerActionAdapter,
    TriggerHandler,
};
pub use metadata::{ActionMetadata, InterfaceVersion, IsolationLevel, MetadataCompatibilityError};
pub use nebula_credential::CredentialGuard;
pub use output::{
    ActionOutput, BinaryData, BinaryStorage, BufferConfig, CacheInfo, Cost, DataReference,
    DeferredOutput, DeferredRetryConfig, DeltaFormat, ExpectedOutput, OutputEnvelope, OutputMeta,
    OutputOrigin, Overflow, PollTarget, Producer, ProducerKind, Progress, Resolution, StreamMode,
    StreamOutput, StreamState, Timing, TokenUsage,
};
pub use port::{ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort};
pub use registry::ActionRegistry;
pub use result::{ActionResult, BranchKey, BreakReason, PortKey, WaitCondition};
pub use scoped::ScopedCredentialAccessor;
pub use testing::{
    SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
    TriggerTestHarness,
};
pub use validation::{
    ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
};

pub use nebula_parameter::{Parameter, ParameterCollection};
