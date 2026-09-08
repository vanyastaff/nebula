//! Admission registry supplied by the plugin crate that owns plugin fixtures.

use std::sync::Arc;

use nebula_plugin::FrozenPluginRegistry;

pub(super) fn registry() -> Arc<FrozenPluginRegistry> {
    nebula_plugin::testing::activation_diagnostic_registry()
}
