use crate::{ParameterCollection, ProcessAction};
use crate::action::ActionContext;
use crate::action::metadata::ActionMetadata;
use crate::connection::ConnectionCollection;
use async_trait::async_trait;
use downcast_rs::{Downcast, impl_downcast};
use dyn_clone::{DynClone, clone_trait_object};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fmt::Debug;
use crate::action::trigger::TriggerAction;

pub trait TypedAction  {
    /// Returns the type of the action
    fn action_type(&self) -> ActionType;
}

#[async_trait]
pub trait Action: TypedAction + DynClone + Downcast + Any + Debug + Send + Sync {

    /// Returns the metadata associated with this action
    fn metadata(&self) -> &ActionMetadata;

    /// Returns the name of the action
    fn name(&self) -> &str {
        self.metadata().name.as_ref()
    }

    /// Returns the unique key of the action
    fn key(&self) -> &str {
        self.metadata().key.as_ref()
    }

    /// Returns the input connections for this action
    fn inputs(&self) -> Option<&ConnectionCollection>;

    /// Returns the output connections for this action
    fn outputs(&self) -> Option<&ConnectionCollection>;

    /// Returns the parameters for this actions
    fn parameters(&self) -> Option<&ParameterCollection>;
}

impl_downcast!(Action);
clone_trait_object!(Action);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionType {
    Process,
    Trigger,
    Polling,
    Webhook,
}
