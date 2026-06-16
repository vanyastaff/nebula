//! # nebula-plugin
//!
//! **Role:** Plugin Distribution Unit — registry + manifest. (plugin
//! is the unit of registration, not the unit of size; full plugins and
//! micro-plugins use the same contract).
//!
//! A plugin bundles Actions, Credentials, and Resources under a versioned
//! identity. This crate provides the trait, manifest types, and the
//! in-memory registry for in-process plugins.
//!
//! ## Key types
//!
//! - `Plugin` — base trait every plugin implements; `actions()`, `credentials()`, `resources()`,
//!   `on_load()`, `on_unload()` (default no-ops). Returns runnable trait objects.
//! - `PluginManifest` — bundle descriptor with builder API (key, name, semver version, group,
//!   `Icon`, maturity, deprecation, author/license/homepage/repository metadata). Does **not**
//!   compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf.
//! - `ResolvedPlugin` — per-plugin wrapper with eager component caches; enforces namespace
//!   invariant at construction.
//! - `PluginRegistry` — in-memory `PluginKey → Arc<ResolvedPlugin>` registry.
//! - `PluginError` — typed error for plugin operations.
//! - `ComponentKind` — discriminant for namespace and duplicate errors.
//! - `#[derive(Plugin)]` — proc-macro derivation.
//!
//! ## Registration contract
//!
//! `impl Plugin` is the single runtime source of truth for what is registered.
//! Do not duplicate `fn actions()` / `fn resources()` / `fn credentials()` in
//! `plugin.toml` — that is spec theater. See `crates/plugin/README.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod dependency;
mod error;
mod manifest;
mod plugin;
pub mod plugin_toml;
mod registry;
mod resolved_plugin;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use dependency::PluginDependencyError;
pub use error::{ComponentKind, PluginError};
pub use manifest::{ManifestError, PluginManifest, PluginManifestBuilder};
// Re-export PluginKey from core for convenience.
pub use nebula_core::PluginKey;
pub use nebula_metadata::PluginDependency;
pub use nebula_plugin_macros::Plugin;
pub use plugin::Plugin;
pub use registry::PluginRegistry;
pub use resolved_plugin::ResolvedPlugin;
