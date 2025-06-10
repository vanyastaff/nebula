
use async_trait::async_trait;
use dyn_clone::DynClone;
use crate::action::ActionError;
use crate::credential::Credential;
use crate::{Key, request::RequestOptionsBuilder};

#[async_trait]
pub trait ActionContext: DynClone + Send + Sync {
    fn get_parameter<T>(&self, key: &str) -> Result<T, ActionError>
    where
        T: serde::de::DeserializeOwned;

    fn get_optional_parameter<T>(&self, key: &str) -> Result<Option<T>, ActionError>
    where
        T: serde::de::DeserializeOwned;

    fn create_request(&self) -> RequestOptionsBuilder {
        RequestOptionsBuilder::default()
    }

    fn get_credential(&self, key: &Key) -> Result<Box<dyn Credential>, ActionError>;

}