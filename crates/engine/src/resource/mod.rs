//! Engine-side resource registration wiring.
//!
//! `ResourceFactory`, `KindActivator`, `RegisterRequest`, `RegistrarError`,
//! `ResourceActivatorRegistry`, and `ResourceRegistrationOutcome` now live in
//! `nebula_resource::factory` (ADR-0095 D2 — moved down into nebula-resource).
//! The engine re-exports them here so engine-internal code reaches them through
//! the existing `crate::resource::*` path without any import changes.
//!
//! The old engine-owned `ResourceActivator` trait name is retired — callers
//! use [`ResourceFactory`].

use std::sync::Arc;

pub mod activation;

pub use activation::{
    ActivatedResource, ActivationContext, ActiveResourceRow, RETIRE_SWEEP_BATCH, RowState,
    StoredResourceActivationError, StoredResourceActivator,
};
pub use nebula_resource::{
    KindActivator, Manager, ManagerConfig, RegisterRequest, RegistrarError,
    ResourceActivatorRegistry, ResourceConfigInput, ResourceFactory, ResourceRegistrationOutcome,
    SlotBinding, rate_limit,
};

/// Why a plugin set could not be turned into a resource allowlist.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResourceWiringError {
    /// Two contributed factories claim the same resource kind. The allowlist
    /// is closed and keyed by kind, so either one would silently shadow the
    /// other; composition fails instead.
    #[error("resource kind `{kind}` is contributed more than once")]
    DuplicateKind {
        /// The contested resource kind.
        kind: String,
    },
    /// A contributed factory's metadata could not be admitted.
    #[error("resource kind `{kind}` has invalid metadata")]
    Metadata {
        /// The rejected resource kind.
        kind: String,
        /// Why admission failed.
        #[source]
        source: nebula_resource::MetadataBuildError,
    },
}

/// Builds the closed `kind → factory` allowlist from every resource factory a
/// plugin set contributes.
///
/// A stored resource row names its `kind` by the resource key, so each
/// factory is inserted under [`ResourceFactory::key`]. Compose one allowlist
/// per process and share it: the engine activates rows through it and the API
/// validates configs through the same instance, so a kind the API accepts is
/// always one the engine can register.
///
/// # Errors
///
/// [`ResourceWiringError::DuplicateKind`] when two factories share a key, and
/// [`ResourceWiringError::Metadata`] when a factory's metadata is invalid.
pub fn resource_registrars_from<'a>(
    factories: impl IntoIterator<Item = &'a Arc<dyn ResourceFactory>>,
) -> Result<ResourceActivatorRegistry, ResourceWiringError> {
    let mut registrars = ResourceActivatorRegistry::new();
    for factory in factories {
        let kind = factory.key().as_str().to_owned();
        if registrars.contains(&kind) {
            return Err(ResourceWiringError::DuplicateKind { kind });
        }
        registrars
            .insert(kind.clone(), Arc::clone(factory))
            .map_err(|source| ResourceWiringError::Metadata { kind, source })?;
    }
    Ok(registrars)
}
