//! Convenience re-exports for action authors.
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//! ```

pub use crate::action::Action;
pub use crate::authoring::{FnStatelessAction, stateless_fn};
pub use crate::capability::{
    ActionLogLevel, ActionLogger, CredentialAccessError, CredentialAccessor, ExecutionEmitter,
    ResourceAccessor, TriggerScheduler,
};
pub use crate::context::{ActionContext, Context, TriggerContext};
pub use crate::dependency::ActionDependencies;
pub use crate::error::{ActionError, ErrorCode};
pub use crate::execution::{ResourceAction, StatelessAction, TriggerAction};
pub use crate::ext::ActionResultExt;
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
pub use crate::registry::ActionRegistry;
pub use crate::result::ActionResult;
pub use crate::stateful::{
    BatchAction, BatchItemResult, PageResult, PaginatedAction, StatefulAction, TransactionalAction,
};
pub use crate::testing::{
    SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
    TriggerTestHarness,
};
pub use crate::trigger::{PollAction, WebhookAction};
pub use crate::validation::{
    ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
};
pub use nebula_credential::CredentialGuard;

pub use nebula_parameter::{Parameter, ParameterCollection};
