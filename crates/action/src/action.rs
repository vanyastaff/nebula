use crate::components::ActionComponents;
use crate::metadata::ActionMetadata;
use nebula_core::deps::FromRegistry;

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
pub trait Action: Send + Sync + 'static {
    /// Statically declared dependencies for this action type.
    ///
    /// Implementations must declare this explicitly using the
    /// [`Requires`](nebula_core::deps::Requires) marker and the
    /// [`deps!`](nebula_core::deps) macro. Use `deps![]` (or `()`) for
    /// actions without dependencies.
    type Deps: FromRegistry;

    /// Static metadata describing this action type.
    fn metadata(&self) -> &ActionMetadata;

    /// Components required by this action.
    fn components(&self) -> ActionComponents;
}
