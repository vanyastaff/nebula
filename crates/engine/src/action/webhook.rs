use async_trait::async_trait;
use crate::action::{Action, ActionContext};

#[async_trait]
pub trait WebhookContext: ActionContext {
    
}

#[async_trait]
pub trait WebhookAction: Action {
    
}