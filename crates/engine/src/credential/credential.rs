use std::any::Any;
use std::fmt::Debug;

use downcast_rs::{Downcast, impl_downcast};
use dyn_clone::{DynClone, clone_trait_object};

use crate::ParameterCollection;
use crate::credential::{CredentialContext, CredentialError, CredentialMetadata};
use crate::request::{RequestOptions, RequestOptionsBuilder};
use crate::types::Key;

#[async_trait::async_trait]
pub trait Credential: DynClone + Downcast + Any + Debug {
    /// Returns the metadata associated with this credential.
    fn metadata(&self) -> &CredentialMetadata;

    /// Returns the name of the credential.
    fn name(&self) -> &str {
        &self.metadata().name
    }

    /// Returns the unique key of the credential.
    fn key(&self) -> &Key {
        &self.metadata().key
    }

    fn parameters(&self) -> ParameterCollection;
}

impl_downcast!(Credential);
clone_trait_object!(Credential);

#[async_trait::async_trait]
pub trait RequestAuthenticator {
    fn authenticate_request(
        &self,
        ctx: Box<dyn CredentialContext>,
        request_builder: RequestOptionsBuilder,
    ) -> Result<RequestOptionsBuilder, CredentialError>;

    async fn test_connection(
        &self,
        ctx: Box<dyn CredentialContext>,
        request_builder: RequestOptionsBuilder,
    ) -> Result<bool, CredentialError>;
}

#[async_trait::async_trait]
pub trait ClientAuthenticator<T> {
    async fn create_authenticated_client(
        &self,
        ctx: Box<dyn CredentialContext>,
    ) -> Result<T, CredentialError>;
    async fn configure_client(
        &self,
        client: &mut T,
        ctx: Box<dyn CredentialContext>,
    ) -> Result<(), CredentialError>;
    async fn test_connection(
        &self,
        ctx: Box<dyn CredentialContext>,
    ) -> Result<bool, CredentialError>;
}


pub struct DadataCredentialInput {
    #[parameter(key = "key", name="API Key", description = "API Key for Dadata", type = ParameterKind::Text, required = true, display = ...)]
    #[validate(required, length(min = 1), regex()]
        "^[a-zA-Z0-9]{32}$",
        message = "API Key must be a 32 character alphanumeric string"
    ))]
    pub api_key: String,
    .....
    pub api_secret: String,
}