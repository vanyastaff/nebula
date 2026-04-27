//! # nebula-action
//!
//! **Role:** Action Trait Family + Execution Policy Metadata (Ports & Adapters).
//! Canon §3.5 (trait family; adding a trait requires canon revision), §11.2, §11.3.
//!
//! Defines what actions are and how they communicate with the engine. Core types
//! live here; execution environments (in-process, ProcessSandbox with capability
//! allowlists and OS-level hardening) are drivers in `nebula-sandbox`.
//! WASM is an explicit non-goal — see `docs/PRODUCT_CANON.md` §12.6.
//!
//! ## Trait family (canon §3.5)
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
//!   `ActionCategory`. NOTE: `CheckpointPolicy` is planned but not yet a field — see
//!   `crates/action/README.md` Contract section.
//! - `ActionResult` — execution result with flow-control intent.
//! - `ActionError` — typed error distinguishing retryable from fatal.
//! - `ActionHandler` — top-level enum dispatcher over all handler variants.
//!
//! See `crates/action/README.md` for the full contract, canon invariants,
//! and CheckpointPolicy status note.

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
/// Error types distinguishing retryable from fatal failures.
pub mod error;
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
    ActionContext, ActionRuntimeContext, CredentialContextExt, HasNodeIdentity,
    HasTriggerScheduling, HasWebhookEndpoint, TriggerContext, TriggerRuntimeContext,
};
pub use control::{ControlAction, ControlActionAdapter, ControlInput, ControlOutcome};
pub use error::{
    ActionError, ActionErrorExt, MAX_VALIDATION_DETAIL, RetryHintCode, ValidationReason,
};
pub use handler::ActionHandler;
pub use idempotency::IdempotencyKey;
pub use metadata::{ActionCategory, ActionMetadata, IsolationLevel, MetadataCompatibilityError};
pub use nebula_action_macros::{Action, action_phantom};
pub use nebula_core::{
    Context, DeclaresDependencies,
    accessor::{EventEmitter, LogLevel, Logger, MetricsEmitter, ResourceAccessor},
    context::{HasCredentials, HasEventBus, HasLogger, HasMetrics, HasResources},
};
pub use nebula_credential::CredentialGuard;
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
pub use result::{
    ActionResult, BranchKey, BreakReason, PortKey, TerminationCode, TerminationReason,
    WaitCondition,
};
pub use stateful::{
    BatchAction, BatchItemResult, BatchState, PageResult, PaginatedAction, PaginationState,
    StatefulAction, StatefulActionAdapter, StatefulHandler,
};
pub use stateless::{
    FnStatelessAction, FnStatelessCtxAction, StatelessAction, StatelessActionAdapter,
    StatelessHandler, stateless_ctx_fn, stateless_fn,
};
pub use testing::{
    SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestActionContext,
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
    DEFAULT_MAX_BODY_BYTES, MAX_HEADER_COUNT, RequiredPolicy, SignatureOutcome, SignaturePolicy,
    SignatureScheme, WebhookAction, WebhookConfig, WebhookEndpointProvider, WebhookHttpResponse,
    WebhookRequest, WebhookResponse, WebhookSource, WebhookTriggerAdapter, hmac_sha256_compute,
    verify_hmac_sha256, verify_hmac_sha256_base64, verify_hmac_sha256_with_timestamp,
    verify_tag_constant_time,
};
