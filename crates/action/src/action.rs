use crate::metadata::{ActionMetadata, ActionType};
use nebula_parameter::collection::ParameterCollection;

/// Base trait for all action types.
///
/// Provides identity and metadata — the engine uses this to inspect
/// capabilities, isolation level, schema, etc. Execution logic is
/// defined by sub-traits ([`ProcessAction`](crate::ProcessAction), etc.).
///
/// # Object Safety
///
/// This trait is object-safe and can be used as `dyn Action`.
/// The engine stores actions as `Arc<dyn Action>` in the registry.
pub trait Action: Send + Sync + 'static {
    /// Static metadata describing this action type.
    fn metadata(&self) -> &ActionMetadata;

    /// The kind of action (Process, Stateful, Trigger, etc.).
    ///
    /// Defaults to the `action_type` field from [`ActionMetadata`].
    /// Override only if the action type is determined dynamically.
    fn action_type(&self) -> ActionType {
        self.metadata().action_type
    }

    /// User-facing parameter definitions, if any.
    ///
    /// Defaults to the parameters from metadata. Override if parameters
    /// are defined separately from metadata.
    fn parameters(&self) -> Option<&ParameterCollection> {
        self.metadata().parameters.as_ref()
    }
}
