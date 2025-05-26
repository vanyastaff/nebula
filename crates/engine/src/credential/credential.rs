use std::any::Any;
use std::fmt::Debug;

use downcast_rs::{Downcast, impl_downcast};
use dyn_clone::{DynClone, clone_trait_object};

use crate::credential::{CredentialContext, CredentialError, CredentialMetadata};
use crate::ParameterCollection;
use crate::request::RequestOptions;
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

    /// Pre-authentication phase for interactive authentication flows
    async fn pre_authenticate(
        &mut self,
        _ctx: Box<dyn CredentialContext>,
        request_options: RequestOptions,
    ) -> Result<RequestOptions, CredentialError> {
        // Default implementation just returns the request options unmodified
        Ok(request_options)
    }

    /// Apply authentication to request options
    fn authenticate(
        &self,
        request_options: RequestOptions,
    ) -> Result<RequestOptions, CredentialError>;

    /// Test the credential
    fn test(&self, request_options: RequestOptions) -> Result<RequestOptions, CredentialError> {
        // Default implementation just calls authenticate
        self.authenticate(request_options)
    }
}

impl_downcast!(Credential);
clone_trait_object!(Credential);
