//! Declarative dependency traits for actions.
use nebula_core::{CredentialKey, ResourceKey};
use nebula_credential::AnyCredential;
use nebula_resource::AnyResource;

/// Declarative dependency declaration for actions.
///
/// Implement this trait to declare which credential and resource dependencies
/// an action requires. The engine calls these methods at registration time.
///
/// Methods use `where Self: Sized` so they are not in the vtable and can
/// only be called on concrete types at registration time.
///
/// # Typed vs. trait-object accessors
///
/// The trait provides two complementary sets of methods:
/// - `credential` / `resources` — return trait objects used for capability injection at runtime.
/// - `credential_keys` / `resource_keys` — return typed [`CredentialKey`] / [`ResourceKey`]
///   values used by the engine for dependency validation before execution begins.
pub trait ActionDependencies {
    /// The credential required by this action, if any.
    fn credential() -> Option<Box<dyn AnyCredential>>
    where
        Self: Sized,
    {
        None
    }

    /// Resources required by this action.
    fn resources() -> Vec<Box<dyn AnyResource>>
    where
        Self: Sized,
    {
        vec![]
    }

    /// Typed credential keys this action requires.
    ///
    /// The engine uses this at registration time to validate that declared
    /// credentials are available in the project before execution begins.
    ///
    /// Returns an empty `Vec` by default (no credential required).
    fn credential_keys() -> Vec<CredentialKey>
    where
        Self: Sized,
    {
        vec![]
    }

    /// Typed resource keys this action requires.
    ///
    /// The engine uses this at registration time to validate that declared
    /// resources are wired up in the workflow graph before execution begins.
    ///
    /// Returns an empty `Vec` by default (no resources required).
    fn resource_keys() -> Vec<ResourceKey>
    where
        Self: Sized,
    {
        vec![]
    }
}
