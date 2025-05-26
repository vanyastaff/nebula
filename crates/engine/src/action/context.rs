use async_trait::async_trait;
use dyn_clone::DynClone;
use crate::action::ActionError;

#[async_trait]
pub trait ActionContext: DynClone + Send + Sync {
    fn get_parameter<T>(&self, key: &str) -> Result<T, ActionError>
    where
        T: serde::de::DeserializeOwned;

    fn get_optional_parameter<T>(&self, key: &str) -> Result<Option<T>, ActionError>
    where
        T: serde::de::DeserializeOwned;
}