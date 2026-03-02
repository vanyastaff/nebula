//! Convenience re-exports for action authors.
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//! ```

pub use crate::action::Action;
pub use crate::components::ActionComponents;
pub use crate::context::{ActionContext, Context, TriggerContext};
pub use crate::execution::{ResourceAction, StatefulAction, StatelessAction, TriggerAction};
pub use crate::error::ActionError;
pub use crate::metadata::ActionMetadata;
pub use crate::output::{
    ActionOutput, DeferredOutput, ExpectedOutput, Producer, ProducerKind, Progress, Resolution,
    StreamMode, StreamOutput,
};
pub use crate::port::{
    ConnectionFilter, DynamicPort, FlowKind, InputPort, OutputPort, SupportPort,
};
pub use crate::result::ActionResult;

pub use nebula_parameter::collection::ParameterCollection;
pub use nebula_parameter::def::ParameterDef;
