//! Reverse-index write helper for `Manager::register`.
//!
//! Splits the `register_inner` body out so that `manager/mod.rs` keeps only
//! the public registration surface and the credential reverse-index machinery
//! lives next to its sibling rotation dispatcher (`manager/rotation.rs`).

use std::{any::TypeId, sync::Arc, time::Duration};

use nebula_core::CredentialId;

use crate::{
    error::Error,
    manager::{
        Manager,
        rotation::{ResourceDispatcher, TypedDispatcher},
    },
    resource::Resource,
    runtime::managed::ManagedResource,
};

impl Manager {
    /// Internal helper: write to `credential_resources` reverse-index when the
    /// resource binds a real credential; no-op for `NoCredential`-bound resources.
    ///
    /// Called by `register<R>` after the registry write succeeds. The `TypeId`
    /// check distinguishes credential-bearing resources from opt-out marker types
    /// at compile time.
    ///
    /// # Errors
    ///
    /// Returns `Error::missing_credential_id` when a credential-bearing resource
    /// (`R::Credential != NoCredential`) is registered without a `credential_id`.
    pub(super) fn register_inner<R: Resource>(
        &self,
        managed: Arc<ManagedResource<R>>,
        credential_id: Option<CredentialId>,
        timeout_override: Option<Duration>,
    ) -> Result<(), Error> {
        let opted_out =
            TypeId::of::<R::Credential>() == TypeId::of::<nebula_credential::NoCredential>();

        match (opted_out, credential_id) {
            (true, Some(_)) => {
                tracing::warn!(
                    resource = %R::key(),
                    "register: NoCredential resource provided a credential_id; ignoring"
                );
            },
            (true, None) => {
                // Normal path for NoCredential-bound resources — no reverse-index write.
            },
            (false, None) => {
                return Err(Error::missing_credential_id(R::key()));
            },
            (false, Some(id)) => {
                let dispatcher: Arc<dyn ResourceDispatcher> =
                    Arc::new(TypedDispatcher::new(managed, timeout_override));
                self.credential_resources
                    .entry(id)
                    .or_default()
                    .push(dispatcher);
            },
        }
        Ok(())
    }
}
