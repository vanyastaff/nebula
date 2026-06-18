//! # nebula-action
//!
//! **Role:** Action Trait Family + Execution Policy Metadata (Ports & Adapters).
//!
//! Defines what actions are and how they communicate with the engine. Core types
//! live here; the engine dispatches actions in-process (`InProcessRunner`).
//! Process/WASM isolation is a non-goal (ADR-0091, canon §12.6).
//! WASM is an explicit non-goal for the action execution surface.
//!
//! ## Trait family
//!
//! - `Action` — base trait providing identity and metadata.
//! - `StatelessAction` — pure, stateless single-execution.
//! - `StatefulAction` — iterative with persistent state (Continue/Break).
//! - `TriggerAction` — workflow starter (start/stop); outside the execution graph.
//! - `ResourceAction` — graph-level DI; configures/cleans up scoped resource.
//! - `PaginatedAction`, `BatchAction` — DX over `StatefulAction`.
//! - `WebhookAction`, `PollAction` — DX over `TriggerAction`.
//! - `ControlAction` — flow-control nodes (If, Switch, Router, NoOp, Stop, Fail).
//!
//! ## Key metadata and result types
//!
//! - `ActionMetadata` — key, version, ports, `ValidSchema` parameters, `IsolationLevel`,
//!   `ActionCategory`, `ActionKind`, and `CheckpointPolicy`.
//! - `ActionResult` — execution result with flow-control intent.
//! - `ActionError` — typed error distinguishing retryable from fatal.
//! - `ActionHandler` — top-level enum dispatcher over all handler variants.
//!
//! See `crates/action/README.md` for the full contract and canon invariants.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Base action trait defining identity and metadata.
pub mod action;
/// Capability interfaces injected into contexts (resources, logger, trigger).
pub mod capability;
/// Runtime context provided to actions during execution.
pub mod context;
/// [`ControlAction`] DX trait, [`ControlOutcome`] / [`ControlInput`] types,
/// and [`ControlActionAdapter`] bridging to [`StatelessHandler`]. The
/// public contract for flow-control nodes (If, Switch, Router, Filter,
/// NoOp, Stop, Fail).
pub mod control;
/// `ErasedAction` enum + per-variant object-safe sub-traits + `ActionFactory`
/// (Phase 3 / Session 4) — engine-side dispatch facade.
pub mod erased;
/// Error types distinguishing retryable from fatal failures.
pub mod error;
/// `ActionFactory` — engine-side per-execution factory that produces a
/// `Box<dyn ErasedAction>` from a workflow node + context.
pub mod factory;
/// `FromWorkflowNode` async factory trait — resolves slot bindings against
/// a workflow node + action context (Phase 3 / Session 2).
pub mod from_workflow_node;
/// Top-level [`ActionHandler`] enum dispatcher. Domain handler traits and
/// adapters live in their respective domain files and are re-exported here
/// for backwards compatibility of the `nebula_action::handler::*` path space.
pub mod handler;
/// [`IdempotencyKey`] — transport-level dedup identifier returned by triggers.
pub mod idempotency;
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
/// `ResourceProduces<R>` — Output marker for `ResourceAction`.
pub mod resource_produces;
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
pub use capability::{ExecutionEmitter, TriggerHealth, TriggerHealthSnapshot, TriggerScheduler};
pub use context::{
    ActionContext, ActionContextExt, ActionRuntimeContext, CredentialContextExt, HasNodeIdentity,
    HasTriggerScheduling, HasWebhookEndpoint, TriggerContext, TriggerRuntimeContext,
};
pub use control::{ControlAction, ControlActionAdapter, ControlInput, ControlOutcome};
pub use erased::{
    ErasedAction, ErasedControl, ErasedResource, ErasedStateful, ErasedStateless, ErasedTrigger,
};
pub use error::{
    ActionError, ActionErrorExt, MAX_VALIDATION_DETAIL, RetryHintCode, ValidationReason,
};
pub use factory::{
    ActionFactory, GenericControlFactory, GenericResourceFactory, GenericStatefulFactory,
    GenericStatelessFactory, GenericTriggerFactory,
};
pub use from_workflow_node::FromWorkflowNode;
pub use handler::ActionHandler;
pub use idempotency::IdempotencyKey;
pub use metadata::{
    ActionCategory, ActionKind, ActionMetadata, CheckpointPolicy, IsolationLevel,
    MetadataCompatibilityError,
};
pub use nebula_action_macros::{Action, action_phantom};
pub use nebula_core::{
    Context, Dependencies,
    accessor::{EventEmitter, LogLevel, Logger, MetricsEmitter, ResourceAccessor},
    context::{HasCredentials, HasEventBus, HasLogger, HasMetrics, HasResources},
};
pub use nebula_credential::{CredentialGuard, CredentialRef};
pub use nebula_resource::ResourceRef;
pub use nebula_schema::{Field, Schema, ValidSchema, field_key};
pub use output::{
    ActionOutput, BinaryData, BinaryStorage, BufferConfig, CacheInfo, Cost, DataReference,
    DeferredOutput, DeferredRetryConfig, DeltaFormat, ExpectedOutput, OutputEnvelope, OutputMeta,
    OutputOrigin, Overflow, PollTarget, Producer, ProducerKind, Progress, Resolution, StreamMode,
    StreamOutput, StreamState, Timing, TokenUsage,
};
pub use poll::{
    DeduplicatingCursor, EmitFailurePolicy, POLL_INTERVAL_FLOOR, PollAction, PollConfig,
    PollCursor, PollOutcome, PollResult, PollSource, PollTriggerAdapter,
};
pub use port::{ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort};
pub use resource::{ResourceAction, ResourceActionAdapter, ResourceHandler};
pub use resource_produces::ResourceProduces;
pub use result::{
    ActionResult, BranchKey, BreakReason, PortKey, TerminationCode, TerminationReason,
    WaitCondition,
};
pub use stateful::{
    BatchAction, BatchItemResult, BatchState, PageResult, PaginatedAction, PaginationState,
    StatefulAction, StatefulActionAdapter, StatefulHandler,
};
pub use stateless::{StatelessAction, StatelessActionAdapter, StatelessHandler};
pub use testing::{
    SpyEmission, SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestActionContext,
    TestContextBuilder, TestTriggerContext, TriggerTestHarness,
};
pub use trigger::{
    TriggerAction, TriggerActionAdapter, TriggerEvent, TriggerEventOutcome, TriggerHandler,
    TriggerSource,
};
pub use validation::{
    ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
};
pub use webhook::{
    BuiltWebhookHandler, Clock, DEFAULT_MAX_BODY_BYTES, FactoryError, MAX_HEADER_COUNT, MockClock,
    PreHandleOutcome, RequiredPolicy, SignatureError, SignatureOutcome, SignaturePolicy,
    SignatureScheme, SystemClock, TimestampFormat, WebhookAction, WebhookActionFactory,
    WebhookActivationSpec, WebhookConfig, WebhookEndpointProvider, WebhookHttpResponse,
    WebhookProvider, WebhookRequest, WebhookResponse, WebhookSource, WebhookTriggerAdapter,
    hmac_sha256_compute, validate_timestamp, verify_hmac_sha256, verify_hmac_sha256_base64,
    verify_hmac_sha256_with_timestamp, verify_tag_constant_time,
};
