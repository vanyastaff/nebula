//! # nebula-plugin
//!
//! **Role:** Plugin Distribution Unit — registry + manifest. Canon §7.1 (plugin
//! is the unit of registration, not the unit of size; full plugins and
//! micro-plugins use the same contract).
//!
//! A plugin bundles Actions, Credentials, and Resources under a versioned
//! identity. This crate provides only the trait, manifest types, and in-memory
//! registry — no I/O, no FFI. Loading and isolation live in `nebula-sandbox`.
//!
//! ## Key types
//!
//! - `Plugin` — base trait every plugin implements; `actions()`, `credentials()`, `resources()`,
//!   `on_load()`, `on_unload()` (default no-ops). Returns runnable trait objects (canon §3.5).
//! - `PluginManifest` — bundle descriptor with builder API (key, name, semver version, group,
//!   `Icon`, maturity, deprecation, author/license/homepage/repository metadata). Does **not**
//!   compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf (ADR-0018).
//! - `ResolvedPlugin` — per-plugin wrapper with eager component caches; enforces namespace
//!   invariant at construction (ADR-0027).
//! - `PluginRegistry` — in-memory `PluginKey → Arc<ResolvedPlugin>` registry.
//! - `PluginError` — typed error for plugin operations.
//! - `ComponentKind` — discriminant for namespace and duplicate errors.
//! - `#[derive(Plugin)]` — proc-macro derivation.
//!
//! ## Canon note — §7.1
//!
//! `impl Plugin` is the single runtime source of truth for what is registered.
//! Do not duplicate `fn actions()` / `fn resources()` / `fn credentials()` in
//! `plugin.toml` — that is spec theater. See `crates/plugin/README.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod manifest;
mod plugin;
mod registry;
pub mod resolved_plugin;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use error::{ComponentKind, PluginError};
pub use manifest::{ManifestError, PluginManifest, PluginManifestBuilder};
// Re-export PluginKey from core for convenience.
pub use nebula_core::PluginKey;
pub use nebula_plugin_macros::Plugin;
pub use plugin::Plugin;
pub use registry::PluginRegistry;
pub use resolved_plugin::ResolvedPlugin;
