use crate::dependency::ActionDependencies;
use crate::metadata::ActionMetadata;

/// Base trait for all action types.
///
/// Provides identity and metadata — the engine uses this to inspect
/// capabilities, isolation level, schema, etc. Execution logic is
/// defined by sub-traits ([`StatelessAction`](crate::StatelessAction), etc.).
///
/// # Object Safety
///
/// This trait is object-safe and can be used as `dyn Action`.
/// The engine stores actions as `Arc<dyn Action>` in the registry.
///
/// Note: [`ActionDependencies`] methods (`credential()`, `resources()`) use
/// `where Self: Sized` and are therefore not part of the vtable. They are
/// called at registration time on concrete types only.
pub trait Action: ActionDependencies + Send + Sync + 'static {
    /// Static metadata describing this action type.
    fn metadata(&self) -> &ActionMetadata;
}
