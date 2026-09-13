//! Convenience re-exports for action authors.
//!
//! Glob-import the prelude to pull in the action trait family, the result and
//! error types, metadata, and the test harness in one line:
//!
//! ```rust
//! use nebula_action::prelude::*;
//!
//! // Everything an action body reaches for is now in scope, e.g.:
//! let result: ActionResult<i32> = ActionResult::success(7);
//! assert!(result.is_success());
//!
//! let err = ActionError::validation("email", ValidationReason::MissingField, None::<String>);
//! assert!(err.is_fatal());
//! ```

pub use nebula_core::{
    Context, Dependencies,
    accessor::{EventEmitter, LogLevel, Logger, MetricsEmitter, ResourceAccessor},
    context::{HasCredentials, HasEventBus, HasLogger, HasMetrics, HasResources},
};
pub use nebula_credential::CredentialGuard;
pub use nebula_schema::{Field, Schema, ValidSchema, field_key};

pub use crate::{
    action::Action,
    agent::AgentAction,
    capability::{ExecutionEmitter, TriggerScheduler},
    context::{
        ActionContext, ActionRuntimeContext, CredentialContextExt, HasNodeIdentity,
        HasTriggerScheduling, TriggerContext, TriggerRuntimeContext,
    },
    control::{ControlAction, ControlActionAdapter, ControlOutcome},
    effect::{
        ActionEffectContract, EffectContractError, EffectFailureCode, EffectInvocationContext,
        EffectInvocationOutcome, EffectPreparationContext, EffectPreparationError,
        EffectQueryContext, EffectReconciliationOutcome, PreparedEffectAdapter,
        PreparedRemoteEffect, ReadOnlyEffectQuery, RemoteDestinationGuarantee, RemoteEffectAction,
        RemoteEffectDescriptor, RemoteEffectFactory, RemoteEffectPolicy, RemoteEffectPolicyError,
        StableKeyGuarantee,
    },
    error::{ActionError, ActionErrorExt, RetryHintCode, ValidationReason},
    idempotency::IdempotencyKey,
    input::{ActionInput, PreparedActionInput},
    metadata::{
        ActionMetadata, ActionMetadataAdmissionError, ActionMetadataDraft,
        ActionMetadataReadmissionError, MetadataCompatibilityError, RecordedActionMetadata,
    },
    output::{
        ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind, Progress, Resolution,
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
    stateless::{StatelessAction, StatelessActionAdapter},
    stream::StreamAction,
    testing::{
        SpyEmission, SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
        TriggerTestHarness,
    },
    trigger::{
        TriggerAction, TriggerActionAdapter, TriggerEvent, TriggerEventOutcome, TriggerSource,
    },
    validation::{ActionPackageValidationError, ActionPackageValidationErrors},
    webhook::{
        SignatureOutcome, WebhookAction, WebhookHttpResponse, WebhookRequest, WebhookResponse,
        WebhookSource, WebhookTriggerAdapter, hmac_sha256_compute, verify_hmac_sha256,
        verify_tag_constant_time,
    },
};
