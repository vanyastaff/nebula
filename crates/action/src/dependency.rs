//! Declarative dependency traits for actions.
use std::any::TypeId;

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
/// - `credential_keys` / `resource_keys` — return typed [`CredentialKey`] / [`ResourceKey`] values
///   used by the engine for dependency validation before execution begins.
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

    /// `TypeId`s of credential types this action requires.
    ///
    /// Used by [`nebula_credential::ScopedCredentialAccessor`] to enforce
    /// that actions can only access credentials they declared.
    /// Populated by `#[derive(Action)]` from `#[action(credential = Type)]`.
    ///
    /// Returns an empty `Vec` by default (no credentials required).
    fn credential_types() -> Vec<TypeId>
    where
        Self: Sized,
    {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DefaultDeps;
    impl ActionDependencies for DefaultDeps {}

    struct CustomDeps;

    struct CredA;
    struct CredB;

    impl ActionDependencies for CustomDeps {
        fn credential_types() -> Vec<TypeId> {
            vec![TypeId::of::<CredA>(), TypeId::of::<CredB>()]
        }
    }

    #[test]
    fn default_credential_types_returns_empty() {
        let types = DefaultDeps::credential_types();
        assert!(types.is_empty());
    }

    #[test]
    fn custom_credential_types_returns_declared_types() {
        let types = CustomDeps::credential_types();
        assert_eq!(types.len(), 2);
        assert!(types.contains(&TypeId::of::<CredA>()));
        assert!(types.contains(&TypeId::of::<CredB>()));
    }
}
