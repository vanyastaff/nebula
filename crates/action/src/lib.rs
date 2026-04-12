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
//! - [`StatelessAction`] — pure, stateless single-execution action
//! - [`StatefulAction`] — iterative action with persistent state (Continue/Break)
//! - [`TriggerAction`] — workflow starter (start/stop), outside execution graph
//! - [`ResourceAction`] — graph-level DI (configure/cleanup), scoped to downstream branch
//! - [`PaginatedAction`] — cursor-driven pagination (DX over StatefulAction)
//! - [`BatchAction`] — fixed-size chunk processing (DX over StatefulAction)
//! - [`WebhookAction`] — webhook lifecycle (DX over TriggerAction)
//! - [`PollAction`] — periodic polling (DX over TriggerAction)
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
/// Capability interfaces injected into contexts (resources, logger, trigger).
pub mod capability;
/// Runtime context provided to actions during execution.
pub mod context;
/// Declarative dependency declaration for actions.
pub mod dependency;
/// Error types distinguishing retryable from fatal failures.
pub mod error;
/// Top-level [`ActionHandler`] enum dispatcher. Domain handler traits and
/// adapters live in their respective domain files and are re-exported here
/// for backwards compatibility of the `nebula_action::handler::*` path space.
pub mod handler;
/// Assertion macros for testing action results (`assert_success!`, etc.).
mod macros;
/// Static metadata, versioning, and execution mode descriptors.
pub mod metadata;
/// Output data representations (inline JSON and blob references).
pub mod output;
/// [`PollAction`] DX trait, [`PollTriggerAdapter`], and poll-specific
/// infrastructure (interval floor, warn throttle, started guard).
pub mod poll;
/// Port definitions describing action input/output connection points.
pub mod port;
/// Convenience re-exports for action authors.
pub mod prelude;
/// [`ResourceAction`] DX trait, [`ResourceHandler`] dyn contract, and adapter.
pub mod resource;
/// Execution result types carrying data and flow-control intent.
pub mod result;
/// [`StatefulAction`] DX trait, [`StatefulHandler`] dyn contract, adapter,
/// and DX patterns (paginated, batch).
pub mod stateful;
/// [`StatelessAction`] DX trait, [`StatelessHandler`] dyn contract, adapter,
/// and function-backed DX adapters.
pub mod stateless;
/// Test utilities for action authors.
pub mod testing;
/// Base [`TriggerAction`] trait, [`TriggerHandler`] dyn contract, the
/// transport-agnostic [`TriggerEvent`] envelope, [`TriggerEventOutcome`],
/// and [`TriggerActionAdapter`]. Webhook and poll specializations (each
/// with their own typed request type) live in [`crate::webhook`] and
/// [`crate::poll`].
pub mod trigger;
/// Action package validation utilities.
pub mod validation;
/// Webhook trigger domain — [`WebhookAction`] DX trait, adapter, and
/// HMAC signature verification primitives.
pub mod webhook;

// ── Public re-exports ───────────────────────────────────────────────────────

pub use action::Action;
pub use capability::{
    ActionLogLevel, ActionLogger, ExecutionEmitter, ResourceAccessor, TriggerScheduler,
};
pub use context::{ActionContext, Context, CredentialContextExt, TriggerContext};
pub use dependency::ActionDependencies;
pub use error::{
    ActionError, ActionErrorExt, MAX_VALIDATION_DETAIL, RetryHintCode, ValidationReason,
};
pub use handler::{ActionHandler, AgentHandler};
pub use metadata::{ActionMetadata, InterfaceVersion, IsolationLevel, MetadataCompatibilityError};
pub use nebula_action_macros::Action;
pub use nebula_credential::CredentialGuard;
pub use nebula_parameter::{Parameter, ParameterCollection};
pub use output::{
    ActionOutput, BinaryData, BinaryStorage, BufferConfig, CacheInfo, Cost, DataReference,
    DeferredOutput, DeferredRetryConfig, DeltaFormat, ExpectedOutput, OutputEnvelope, OutputMeta,
    OutputOrigin, Overflow, PollTarget, Producer, ProducerKind, Progress, Resolution, StreamMode,
    StreamOutput, StreamState, Timing, TokenUsage,
};
pub use poll::{
    DeduplicatingCursor, EmitFailurePolicy, POLL_INTERVAL_FLOOR, PollAction, PollConfig,
    PollCursor, PollResult, PollTriggerAdapter,
};
pub use port::{ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort};
pub use resource::{ResourceAction, ResourceActionAdapter, ResourceHandler};
pub use result::{ActionResult, BranchKey, BreakReason, PortKey, WaitCondition};
pub use stateful::{
    BatchAction, BatchItemResult, BatchState, PageResult, PaginatedAction, PaginationState,
    StatefulAction, StatefulActionAdapter, StatefulHandler,
};
pub use stateless::{
    FnStatelessAction, FnStatelessCtxAction, StatelessAction, StatelessActionAdapter,
    StatelessHandler, stateless_ctx_fn, stateless_fn,
};
pub use testing::{
    SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
    TriggerTestHarness,
};
pub use trigger::{
    TriggerAction, TriggerActionAdapter, TriggerEvent, TriggerEventOutcome, TriggerHandler,
};
pub use validation::{
    ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
};
pub use webhook::{
    DEFAULT_MAX_BODY_BYTES, MAX_HEADER_COUNT, SignatureOutcome, WebhookAction, WebhookHttpResponse,
    WebhookRequest, WebhookResponse, WebhookTriggerAdapter, hmac_sha256_compute,
    verify_hmac_sha256, verify_tag_constant_time,
};
