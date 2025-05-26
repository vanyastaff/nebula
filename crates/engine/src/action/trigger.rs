use async_trait::async_trait;
use crate::action::{Action, ActionContext};

#[async_trait]
pub trait TriggerContext: ActionContext {}

#[async_trait]
pub trait TriggerAction: Action {}