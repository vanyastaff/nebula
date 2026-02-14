//! Action handler trait for dynamic dispatch.
//!
//! The runtime stores action handlers in the registry as trait objects.
//! Each handler knows how to execute a specific action given JSON input.

use async_trait::async_trait;
use nebula_action::{ActionError, ActionMetadata};

/// A handler that can execute an action with JSON input/output.
///
/// This trait bridges the typed `ProcessAction`/`StatefulAction` traits
/// and the runtime's JSON-based execution. Implementations typically
/// wrap a concrete action and handle serialization.
///
/// Stored in the [`ActionRegistry`](crate::ActionRegistry) as
/// `Arc<dyn ActionHandler>`.
#[async_trait]
pub trait ActionHandler: Send + Sync {
    /// Execute the action with JSON input and return JSON output.
    async fn execute(
        &self,
        input: serde_json::Value,
        context: nebula_action::context::ActionContext,
    ) -> Result<serde_json::Value, ActionError>;

    /// Static metadata for this action.
    fn metadata(&self) -> &ActionMetadata;
}
