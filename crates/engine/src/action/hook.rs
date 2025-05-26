use async_trait::async_trait;
use crate::action::{Action, ActionContext};

#[async_trait]
pub trait HookContext: ActionContext {
    
}

#[async_trait]
pub trait HookAction: Action {
    
}