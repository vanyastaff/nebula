//! Convenience re-exports for action authors.
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//! ```

pub use crate::action::Action;
pub use crate::capability::{
    ActionLogLevel, ActionLogger, ExecutionEmitter, ResourceAccessor, TriggerScheduler,
};
pub use crate::context::{ActionContext, Context, CredentialContextExt, TriggerContext};
pub use crate::dependency::ActionDependencies;
pub use crate::error::{ActionError, ActionErrorExt, RetryHintCode, ValidationReason};
pub use crate::handler::{
    ActionHandler, IncomingEvent, ResourceActionAdapter, StatefulActionAdapter,
    StatelessActionAdapter, TriggerActionAdapter, TriggerEventOutcome,
};
pub use crate::metadata::{ActionMetadata, MetadataCompatibilityError};
pub use crate::output::{
    ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind, Progress, Resolution,
    StreamMode, StreamOutput,
};
pub use crate::port::{
    ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort,
};
pub use crate::resource::ResourceAction;
pub use crate::result::ActionResult;
pub use crate::stateful::{
    BatchAction, BatchItemResult, PageResult, PaginatedAction, StatefulAction,
};
pub use crate::stateless::{FnStatelessAction, StatelessAction, stateless_fn};
pub use crate::testing::{
    SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
    TriggerTestHarness,
};
pub use crate::trigger::{PollAction, TriggerAction, WebhookAction};
pub use crate::validation::{
    ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
};
pub use crate::webhook::{
    SignatureOutcome, hmac_sha256_compute, verify_hmac_sha256, verify_tag_constant_time,
};
pub use nebula_credential::CredentialGuard;
pub use nebula_parameter::{Parameter, ParameterCollection};
