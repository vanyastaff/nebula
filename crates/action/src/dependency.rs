//! Declarative dependency traits for actions.
use nebula_credential::AnyCredential;
use nebula_resource::AnyResource;

/// Declarative dependency declaration for actions.
///
/// Implement this trait to declare which credential and resource dependencies
/// an action requires. The engine calls these methods at registration time.
///
/// Methods use `where Self: Sized` so they are not in the vtable and can
/// only be called on concrete types at registration time.
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
}
