//! Convenience re-exports for action authors.
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//! ```

pub use crate::action::Action;
pub use crate::context::ActionContext;
pub use crate::provider::{ActionLogger, ActionMetrics, CredentialProvider, SecureString};
pub use crate::error::ActionError;
pub use crate::metadata::{ActionMetadata, ActionType};
pub use crate::output::{
    ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind, Progress, Resolution,
    StreamMode, StreamOutput,
};
pub use crate::port::{
    ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort,
};
pub use crate::result::ActionResult;
pub use crate::types::InteractiveAction;
pub use crate::types::ProcessAction;
pub use crate::types::SimpleAction;
pub use crate::types::StatefulAction;
pub use crate::types::StreamingAction;
pub use crate::types::TransactionalAction;
pub use crate::types::TriggerAction;

pub use nebula_parameter::collection::ParameterCollection;
pub use nebula_parameter::def::ParameterDef;
