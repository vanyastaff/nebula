//! Convenience re-exports for action authors.
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//! ```

pub use nebula_core::{
    Context, DeclaresDependencies,
    accessor::{EventEmitter, LogLevel, Logger, MetricsEmitter, ResourceAccessor},
    context::{HasCredentials, HasEventBus, HasLogger, HasMetrics, HasResources},
};
pub use nebula_credential::CredentialGuard;
pub use nebula_schema::{Field, Schema, ValidSchema, field_key};

pub use crate::{
    action::Action,
    capability::{ExecutionEmitter, TriggerScheduler},
    context::{
        ActionContext, ActionRuntimeContext, CredentialContextExt, HasNodeIdentity,
        HasTriggerScheduling, TriggerContext, TriggerRuntimeContext,
    },
    control::{ControlAction, ControlActionAdapter, ControlInput, ControlOutcome},
    error::{ActionError, ActionErrorExt, RetryHintCode, ValidationReason},
    handler::ActionHandler,
    idempotency::IdempotencyKey,
    metadata::{ActionMetadata, MetadataCompatibilityError},
    output::{
        ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind, Progress, Resolution,
        StreamMode, StreamOutput,
    },
    poll::{
        DeduplicatingCursor, EmitFailurePolicy, PollAction, PollConfig, PollCursor, PollOutcome,
        PollResult, PollSource, PollTriggerAdapter,
    },
    port::{ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort},
    resource::{ResourceAction, ResourceActionAdapter},
    result::{ActionResult, TerminationCode, TerminationReason},
    stateful::{
        BatchAction, BatchItemResult, PageResult, PaginatedAction, StatefulAction,
        StatefulActionAdapter,
    },
    stateless::{FnStatelessAction, StatelessAction, StatelessActionAdapter, stateless_fn},
    testing::{
        SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
        TriggerTestHarness,
    },
    trigger::{
        TriggerAction, TriggerActionAdapter, TriggerEvent, TriggerEventOutcome, TriggerSource,
    },
    validation::{
        ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
    },
    webhook::{
        SignatureOutcome, WebhookAction, WebhookHttpResponse, WebhookRequest, WebhookResponse,
        WebhookSource, WebhookTriggerAdapter, hmac_sha256_compute, verify_hmac_sha256,
        verify_tag_constant_time,
    },
};
