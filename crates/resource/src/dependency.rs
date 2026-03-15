//! Declarative dependency traits for resources.

use crate::any::AnyResource;

/// Declarative dependency declaration for resources.
///
/// Implement this trait on a `Resource` type to declare which
/// sub-resource dependencies it requires. The engine calls these methods
/// at registration time to build the dependency graph automatically.
///
/// Methods use `where Self: Sized` so they are not in the vtable and can
/// only be called on concrete types at registration time.
pub trait ResourceDependencies {
    /// Sub-resources required by this resource.
    fn resources() -> Vec<Box<dyn AnyResource>>
    where
        Self: Sized,
    {
        vec![]
    }
}
