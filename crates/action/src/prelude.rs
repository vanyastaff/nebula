//! Convenience re-exports for action authors.
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//! ```

pub use nebula_credential::CredentialGuard;
pub use nebula_parameter::{Parameter, ParameterCollection};

pub use crate::{
    action::Action,
    capability::{
        ActionLogLevel, ActionLogger, ExecutionEmitter, ResourceAccessor, TriggerScheduler,
    },
    context::{ActionContext, Context, CredentialContextExt, TriggerContext},
    dependency::ActionDependencies,
    error::{ActionError, ActionErrorExt, RetryHintCode, ValidationReason},
    handler::ActionHandler,
    metadata::{ActionMetadata, MetadataCompatibilityError},
    output::{
        ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind, Progress, Resolution,
        StreamMode, StreamOutput,
    },
    poll::{
        DeduplicatingCursor, EmitFailurePolicy, PollAction, PollConfig, PollResult,
        PollTriggerAdapter,
    },
    port::{ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort},
    resource::{ResourceAction, ResourceActionAdapter},
    result::ActionResult,
    stateful::{
        BatchAction, BatchItemResult, PageResult, PaginatedAction, StatefulAction,
        StatefulActionAdapter,
    },
    stateless::{FnStatelessAction, StatelessAction, StatelessActionAdapter, stateless_fn},
    testing::{
        SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
        TriggerTestHarness,
    },
    trigger::{TriggerAction, TriggerActionAdapter, TriggerEvent, TriggerEventOutcome},
    validation::{
        ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
    },
    webhook::{
        SignatureOutcome, WebhookAction, WebhookHttpResponse, WebhookRequest, WebhookResponse,
        WebhookTriggerAdapter, hmac_sha256_compute, verify_hmac_sha256, verify_tag_constant_time,
    },
};
