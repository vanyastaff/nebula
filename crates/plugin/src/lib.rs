//! # Nebula Plugin
//!
//! Plugin system for the Nebula workflow engine.
//!
//! A **plugin** is the user-visible, versionable packaging unit in Nebula — for
//! example "Slack", "HTTP Request", or "PostgreSQL". Each plugin bundles:
//!
//! - [`PluginMetadata`] — key, name, version, group, icon, docs URL
//! - [`PluginComponents`] — registered actions and credential requirements
//!
//! ## Core Types
//!
//! - [`Plugin`] — base trait every plugin implements
//! - [`PluginMetadata`] — static descriptor with builder API
//! - [`PluginComponents`] — runtime component collection (actions, credentials)
//! - [`PluginType`] — enum wrapping a single plugin or a versioned set
//! - [`PluginVersions`] — multi-version container keyed by `u32`
//! - [`PluginRegistry`] — in-memory registry mapping [`PluginKey`] → [`PluginType`]
//! - [`PluginError`] — error type for plugin operations
//!
//! ## Dynamic Loading (feature-gated)
//!
//! With the `dynamic-loading` feature enabled, [`PluginLoader`] can load
//! plugins from shared libraries (`.dll` / `.so` / `.dylib`).

// `deny` instead of `forbid` so the `loader` module can use `allow(unsafe_code)` for FFI.
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod components;
mod error;
#[cfg(feature = "dynamic-loading")]
mod loader;
mod metadata;
mod plugin;
mod plugin_type;
mod registry;
mod versions;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use components::PluginComponents;
pub use error::PluginError;
#[cfg(feature = "dynamic-loading")]
pub use loader::{PluginLoadError, PluginLoader};
pub use metadata::PluginMetadata;
pub use plugin::Plugin;
pub use plugin_type::PluginType;
pub use registry::PluginRegistry;
pub use versions::PluginVersions;

// Re-export PluginKey from core for convenience.
pub use nebula_core::PluginKey;
