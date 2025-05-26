use crate::action::metadata::ActionMetadata;
use crate::ParameterCollection;
use async_trait::async_trait;
use downcast_rs::{impl_downcast, Downcast};
use dyn_clone::{ DynClone, clone_trait_object };
use std::any::Any;
use std::fmt::Debug;
use crate::action::ActionContext;
use crate::connection::Connections;

#[async_trait]
pub trait Action: DynClone + Downcast + Any + Debug + Send + Sync {
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
    
    /// Returns the type of the action
    fn action_type(&self) -> ActionType;

    /// Returns the input connections for this action
    fn inputs(&self) -> Option<&Connections>;

    /// Returns the output connections for this action
    fn outputs(&self) -> Option<&Connections>;

    /// Returns the parameters for this actions
    fn parameters(&self) -> Option<&ParameterCollection>;
}


impl_downcast!(Action);
clone_trait_object!(Action);


#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionType {
    Executable,
    Trigger,
    Polling,
    Hook,
    Webhook,
}

