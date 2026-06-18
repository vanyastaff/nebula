//! Plugin → engine wiring: bridge a `ResolvedPlugin` into the engine's
//! executable `ActionRegistry`.
//!
//! `WorkflowEngine::with_plugin` is the single entry point. It registers
//! every action factory the plugin declares into the engine's live
//! `ActionRegistry` (making the actions dispatchable) **and** records the
//! plugin in the engine's `PluginRegistry` (making its metadata queryable).
//!
//! ## Load ordering
//!
//! `Plugin::on_load` runs **before** any factory or plugin-registry mutation.
//! A failing `on_load` aborts wiring with nothing registered — the engine
//! state is unchanged. `Plugin::on_unload` and rollback of a later-step
//! failure (e.g. `ActionRegistry` mutation) require an `InstallTxn` abstraction;
//! that is a named deferral beyond this bridge.
//!
//! ## Out of scope (deliberate deferral)
//!
//! - **Resource wiring**: `Plugin::resources()` yields `Arc<dyn ResourceFactory>`
//!   which carries introspection but not the typed `R + R::Topology` construction
//!   surface the `ResourceActivatorRegistry` needs. Resource wiring requires a
//!   per-kind `KindActivator` supplied by the composition root.
//! - **Credential wiring**: credential kinds need a separate registration path
//!   not yet exposed on the engine builder.
//! - **Unload / removal**: `ActionRegistry` has no removal primitive. Unload
//!   support requires an ADR decision on hot-reload safety.

use nebula_core::ActionKey;
use nebula_plugin::{PluginError, PluginKey};

/// Error returned when wiring a plugin into the engine fails.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginWiringError {
    /// The plugin key is already registered in the engine's plugin registry.
    ///
    /// Registering the same plugin twice is rejected rather than silently
    /// replacing it, because an in-flight execution may be dispatching against
    /// the existing factory set; silent replacement would create a race window.
    #[error(
        "plugin '{plugin_key}' is already registered in the engine; \
         unload is not yet supported (see ADR-0095)"
    )]
    DuplicatePlugin {
        /// The conflicting plugin key.
        plugin_key: PluginKey,
    },

    /// An action key contributed by this plugin conflicts with an action already
    /// registered in the `ActionRegistry`.
    ///
    /// Unlike `ActionRegistry::register_factory` (which replaces on same
    /// `key+version`), `with_plugin` treats any pre-existing entry as a wiring
    /// fault: two plugins must not claim the same action key, and a plugin must
    /// not be partially registered if any of its actions conflict.
    #[error(
        "action key '{action_key}' from plugin '{plugin_key}' conflicts with \
         an already-registered action in the engine's ActionRegistry"
    )]
    DuplicateActionKey {
        /// The plugin declaring the conflicting action.
        plugin_key: PluginKey,
        /// The conflicting action key.
        action_key: ActionKey,
    },

    /// `Plugin::on_load` returned an error.
    ///
    /// The engine state is unchanged: no action factories were registered and
    /// the plugin was not recorded in the plugin registry. The plugin should
    /// be considered unloaded.
    #[error("plugin '{plugin_key}' on_load hook failed: {source}")]
    OnLoad {
        /// The plugin whose `on_load` failed.
        plugin_key: PluginKey,
        /// The underlying plugin error.
        #[source]
        source: PluginError,
    },
}
