//! Declarative dependency traits for resources.
use nebula_credential::AnyCredential;

use crate::any::AnyResource;

/// Declarative dependency declaration for resources.
///
/// Implement this trait on a `Resource` type to declare which credential
/// and sub-resource dependencies it requires. The engine calls these methods
/// at registration time to build the dependency graph automatically.
///
/// Methods use `where Self: Sized` so they are not in the vtable and can
/// only be called on concrete types at registration time.
pub trait ResourceDependencies {
    /// The credential required by this resource, if any.
    fn credential() -> Option<Box<dyn AnyCredential>>
    where
        Self: Sized,
    {
        None
    }

    /// Sub-resources required by this resource.
    fn resources() -> Vec<Box<dyn AnyResource>>
    where
        Self: Sized,
    {
        vec![]
    }
}
