use crate::action::{Action, ActionContext, ActionError, ActionResult, ActionType, TypedAction};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait ProcessContext: ActionContext {}

#[async_trait]
pub trait ProcessAction: Action {
    type Input: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de>;
    type Output: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema;

    async fn execute<C>(
        &self,
        context: &C,
        input: Self::Input,
    ) -> Result<ActionResult<Self::Output>, ActionError>
    where
        C: ProcessContext + Send + Sync;

    async fn rollback<C>(&self, context: &C, input: Self::Input) -> Result<(), ActionError>
    where
        C: ProcessContext + Send + Sync,
    {
        Ok(())
    }

    fn supports_rollback(&self) -> bool {
        false
    }
}

impl<T: ProcessAction> TypedAction for T {
    fn action_type(&self) -> ActionType {
        ActionType::Process
    }
}

pub struct ProcessExecutor;

impl ProcessExecutor {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute<A, C>(
        &self,
        action: &A,
        context: &C,
        input: A::Input,
    ) -> Result<ActionResult<A::Output>, ActionError>
    where
        A: ProcessAction + Send + Sync,
        A::Input: 'static,
        A::Output: 'static,
        C: ProcessContext + Send + Sync,
    {
        action.execute(context, input).await
    }

    pub fn rollback<A, C>(
        &self,
        action: &A,
        context: &C,
        input: A::Input,
    ) -> Result<(), ActionError>
    where
        A: ProcessAction + Send + Sync,
        A::Input: 'static,
        C: ProcessContext + Send + Sync,
    {
        if action.supports_rollback() {
            action.rollback(context, input)
        } else {
            Err(ActionError::RollbackNotSupported)
        }
    }
}
