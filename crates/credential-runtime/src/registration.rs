//! Composite credential registration — atomically register a credential
//! type into all three registries (CredentialRegistry / StateProjection /
//! CredentialDispatch). Pre-check all, then commit. Closes the drift
//! vector that Task 19's invariant probe surfaces post-facto.

use nebula_credential::{
    Credential, CredentialRegistry, CredentialState, contract::plugin_capability_report,
};
use nebula_engine::credential::StateProjectionRegistry;

use crate::dispatch::CredentialDispatch;

/// Failure modes for atomic composite registration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistrationError {
    /// A credential KEY collided in [`CredentialRegistry`] or
    /// [`CredentialDispatch`]. Operators resolve via namespace fix or
    /// plugin uninstall.
    #[error("credential key `{key}` already registered in `{registry}`")]
    DuplicateKey {
        /// The colliding key.
        key: &'static str,
        /// Which registry detected the collision.
        registry: &'static str,
    },

    /// A state KIND collided in [`StateProjectionRegistry`].
    #[error("state kind `{kind}` already registered in state-projection registry")]
    DuplicateKind {
        /// The colliding state kind.
        kind: &'static str,
    },
}

/// Atomically register a credential type into all three registries.
///
/// Two-phase: pre-check each registry for collision via the `contains`
/// accessor, then commit all three. On collision, no partial state is left
/// in any registry (the pre-check confirms all slots are free before any
/// write). Given single-threaded plugin init this is effectively atomic;
/// this function is not designed for concurrent registration of the same key.
///
/// # Errors
///
/// [`RegistrationError::DuplicateKey`] if `C::KEY` is already in
/// `CredentialRegistry` or `CredentialDispatch`.
/// [`RegistrationError::DuplicateKind`] if `<C::State as CredentialState>::KIND`
/// is already in `StateProjectionRegistry`.
pub fn register_credential_complete<C>(
    credential_registry: &mut CredentialRegistry,
    state_projection: &mut StateProjectionRegistry,
    dispatch: &mut CredentialDispatch,
    instance: C,
    registering_crate: &'static str,
) -> Result<(), RegistrationError>
where
    C: Credential
        + plugin_capability_report::IsInteractive
        + plugin_capability_report::IsRefreshable
        + plugin_capability_report::IsRevocable
        + plugin_capability_report::IsTestable
        + plugin_capability_report::IsDynamic
        + 'static,
    C::Scheme: 'static,
{
    let key = C::KEY;
    let kind = <C::State as CredentialState>::KIND;

    // Phase 1 — peek each registry without registering.
    if credential_registry.contains(key) {
        return Err(RegistrationError::DuplicateKey {
            key,
            registry: "CredentialRegistry",
        });
    }
    if state_projection.contains(kind) {
        return Err(RegistrationError::DuplicateKind { kind });
    }
    if dispatch.contains(key) {
        return Err(RegistrationError::DuplicateKey {
            key,
            registry: "CredentialDispatch",
        });
    }

    // Phase 2 — commit. Pre-check confirmed slots are free.
    credential_registry
        .register(instance, registering_crate)
        .map_err(|_| RegistrationError::DuplicateKey {
            key,
            registry: "CredentialRegistry",
        })?;
    state_projection
        .register::<C>()
        .map_err(|_| RegistrationError::DuplicateKind { kind })?;
    dispatch
        .register::<C>()
        .map_err(|_| RegistrationError::DuplicateKey {
            key,
            registry: "CredentialDispatch",
        })?;

    Ok(())
}
