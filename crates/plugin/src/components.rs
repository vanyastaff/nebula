//! Plugin component collection for action and credential registration.

use std::sync::Arc;

use async_trait::async_trait;
// TODO: These types are currently unavailable in nebula-action
// use nebula_action::ProcessAction;
// use nebula_action::StatefulAction;
// use nebula_action::StatefulActionAdapter;
// use nebula_action::TriggerAction;
// // TriggerActionAdapter is deprecated - triggers now use TriggerContext
// use nebula_action::adapters::ProcessActionAdapter;
// use nebula_action::handler::InternalHandler;
use nebula_credential::CredentialDescription;
// use serde::Serialize;
// use serde::de::DeserializeOwned;

// Temporary placeholder for InternalHandler
/// Temporary handler trait until types are restored
#[async_trait]
pub trait InternalHandler: Send + Sync {
    /// Get action metadata
    fn metadata(&self) -> &nebula_action::ActionMetadata;
    /// Execute the action (placeholder)
    async fn execute(
        &self,
        input: serde_json::Value,
        context: nebula_action::NodeContext,
    ) -> Result<nebula_action::ActionResult<serde_json::Value>, nebula_action::ActionError>;
}

/// Collects the runtime components (actions, credentials) registered by a plugin.
///
/// During [`Plugin::register()`](crate::Plugin::register), plugins add their typed
/// actions and credential requirements. The runtime then extracts handlers
/// for the action registry.
pub struct PluginComponents {
    credentials: Vec<CredentialDescription>,
    handlers: Vec<Arc<dyn InternalHandler>>,
}

impl PluginComponents {
    /// Create an empty collection.
    pub fn new() -> Self {
        Self {
            credentials: Vec::new(),
            handlers: Vec::new(),
        }
    }

    /// Add a credential description.
    pub fn credential(&mut self, desc: CredentialDescription) -> &mut Self {
        self.credentials.push(desc);
        self
    }

    // TODO: These methods are disabled until action types are restored

    // /// Register a typed [`ProcessAction`].
    // ///
    // /// The action is wrapped in a [`ProcessActionAdapter`] that handles
    // /// JSON-to-typed conversion automatically.
    // pub fn process_action<A>(&mut self, action: A) -> &mut Self
    // where
    //     A: ProcessAction + Send + Sync + 'static,
    //     A::Input: DeserializeOwned + Send + Sync + 'static,
    //     A::Output: Serialize + Send + Sync + 'static,
    // {
    //     self.handlers
    //         .push(Arc::new(ProcessActionAdapter::new(action)));
    //     self
    // }

    // /// Register a typed [`StatefulAction`].
    // ///
    // /// The action is wrapped in a [`StatefulActionAdapter`] that handles
    // /// JSON-to-typed conversion and the Continue/Break loop automatically.
    // pub fn stateful_action<A>(&mut self, action: A) -> &mut Self
    // where
    //     A: StatefulAction + Send + Sync + 'static,
    //     A::Input: DeserializeOwned + Clone + Send + Sync + 'static,
    //     A::Output: Serialize + Send + Sync + 'static,
    //     A::State: Send + Sync + 'static,
    // {
    //     self.handlers
    //         .push(Arc::new(StatefulActionAdapter::new(action)));
    //     self
    // }

    // /// Register a typed [`TriggerAction`].
    // ///
    // /// NOTE: Triggers now use TriggerContext and are NOT registered via adapters.
    // /// This method is deprecated and will panic.
    // #[deprecated(note = "Triggers now use TriggerContext. Register directly with TriggerManager")]
    // pub fn trigger_action<A>(&mut self, _action: A) -> &mut Self
    // where
    //     A: TriggerAction + Send + Sync + 'static,
    //     A::Config: DeserializeOwned + Send + Sync + 'static,
    //     A::Event: Serialize + Send + Sync + 'static,
    // {
    //     panic!(
    //         "trigger_action is deprecated. Triggers now use TriggerContext and should be registered with TriggerManager"
    //     );
    // }

    /// Add a pre-built internal handler directly.
    pub fn handler(&mut self, handler: Arc<dyn InternalHandler>) -> &mut Self {
        self.handlers.push(handler);
        self
    }

    /// The registered credential descriptions.
    pub fn credentials(&self) -> &[CredentialDescription] {
        &self.credentials
    }

    /// The registered internal handlers.
    pub fn handlers(&self) -> &[Arc<dyn InternalHandler>] {
        &self.handlers
    }

    /// Consume and split into parts.
    pub fn into_parts(self) -> (Vec<CredentialDescription>, Vec<Arc<dyn InternalHandler>>) {
        (self.credentials, self.handlers)
    }
}

impl Default for PluginComponents {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PluginComponents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginComponents")
            .field("credentials", &self.credentials.len())
            .field("handlers", &self.handlers.len())
            .finish()
    }
}
