use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::action::{Action, ActionContext, ActionError};
use crate::action::polling::{PollingContext, PollingResult};
use crate::action::result::ActionResult;

#[async_trait]
pub trait TriggerContext: ActionContext {
    fn emit();
    fn emit_error(&self, error: ActionError);
    
    
}

#[async_trait]
pub trait TriggerAction: Action {
    type Input: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de>;
    type Output: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de> + J;

    async fn trigger<C>(&self, context: &C, input: Self::Input) -> Result<PollingResult<Self::Output>, ActionError>
    where
        C: TriggerContext + Send + Sync;
}