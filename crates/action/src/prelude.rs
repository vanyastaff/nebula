//! Convenience re-exports for action authors.
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//! ```

pub use crate::action::Action;
pub use crate::authoring::{FnStatelessAction, stateless_fn};
pub use crate::capability::{
    ActionLogLevel, ActionLogger, CredentialAccessor, ExecutionEmitter, ResourceAccessor,
    TriggerScheduler,
};
pub use crate::components::ActionComponents;
pub use crate::context::{ActionContext, Context, TriggerContext};
pub use crate::error::ActionError;
pub use crate::execution::{ResourceAction, StatefulAction, StatelessAction, TriggerAction};
pub use crate::metadata::{ActionMetadata, MetadataCompatibilityError};
pub use crate::output::{
    ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind, Progress, Resolution,
    StreamMode, StreamOutput,
};
pub use crate::port::{
    ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort,
};
pub use crate::reference::ActionRef;
pub use crate::result::ActionResult;
pub use crate::validation::{
    ActionPackageValidationError, ActionPackageValidationErrors, validate_action_package,
};

pub use nebula_parameter::schema::{Field, Schema};
