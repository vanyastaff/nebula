//! # Nebula Plugin
//!
//! Plugin system for the Nebula workflow engine.
//!
//! A **plugin** is the user-visible, versionable packaging unit in Nebula — for
//! example "Slack", "HTTP Request", or "PostgreSQL". Each plugin bundles:
//!
//! - [`PluginMetadata`] — key, name, version, group, icon, docs URL
//! - [`ActionDescriptor`], [`CredentialDescriptor`], [`ResourceDescriptor`] — lightweight
//!   descriptors declaring what the plugin provides, returned by the [`Plugin`] trait methods
//!   `actions()`, `credentials()`, and `resources()` respectively
//!
//! ## Core Types
//!
//! - [`Plugin`] — base trait every plugin implements; provides `actions()`, `credentials()`,
//!   `resources()`, `on_load()`, and `on_unload()` with default no-op implementations
//! - [`PluginMetadata`] — static descriptor with builder API
//! - [`PluginType`] — enum wrapping a single plugin or a versioned set
//! - [`PluginVersions`] — multi-version container keyed by `u32`
//! - [`PluginRegistry`] — in-memory registry mapping [`PluginKey`] → [`PluginType`]
//! - [`PluginError`] — error type for plugin operations
//!
//! ## Plugin Loading
//!
//! Plugin loading (WASM sandbox) is handled by the `nebula-sandbox` crate.
//! This crate provides only the trait, metadata, and registry — no I/O or FFI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod descriptor;
mod error;
mod metadata;
mod plugin;
mod plugin_type;
mod registry;
mod versions;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use descriptor::{ActionDescriptor, CredentialDescriptor, ResourceDescriptor};
pub use error::PluginError;
pub use metadata::PluginMetadata;
// Re-export PluginKey from core for convenience.
pub use nebula_core::PluginKey;
pub use nebula_plugin_macros::Plugin;
pub use plugin::Plugin;
pub use plugin_type::PluginType;
pub use registry::PluginRegistry;
pub use versions::PluginVersions;
