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
//! - `FrozenPluginRegistry`, `PluginSet`, and `WorkerFlavorRevision` — immutable activation
//!   vocabulary.
//! - `ExecutablePlanRevision` — opaque, authority-free Graph-v1 output compiled only from a
//!   frozen registry and an exact workflow revision.
//! - `PluginError` — typed error for plugin operations.
//! - `ComponentKind` — discriminant for namespace and duplicate errors.
//! - `#[derive(Plugin)]` — proc-macro derivation.
//!
//! ## Registration contract
//!
//! `impl Plugin` is the single runtime source of truth for what is registered.
//! Do not duplicate `fn actions()` / `fn resources()` / `fn credentials()` in
//! `plugin.toml` — that is spec theater. See `crates/plugin/README.md`.
//!
//! ## Immutable activation boundary
//!
//! ADR-0115 identity and immutability vocabulary and the pure Graph-v1 compiler are
//! default-public. Retained exact loading, engine dispatch, API transport, persisted routing, and
//! admission have not migrated to them end to end, so this remains a partial closed epoch with
//! zero production consumer rather than an operational capability. A `PluginSetId` is an
//! independent pin, not proof of schemas, runtime behavior, artifact authenticity, authorization,
//! or a complete frozen registry.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod compatibility;
mod compiler;
mod dependency;
mod error;
mod flavor;
mod flavor_context;
mod manifest;
mod plan;
mod plugin;
pub mod plugin_toml;
mod registry;
mod resolved_plugin;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use dependency::PluginDependencyError;
pub use error::{ComponentKind, PluginError};
pub use flavor::{
    PluginContractDescriptor, PluginSet, RuntimeContractVersion, RuntimeContractVersionError,
    WorkerFlavorRevision,
};
pub use flavor_context::WorkerFlavorContext;
pub use manifest::{ManifestError, PluginManifest, PluginManifestBuilder};
// Re-export PluginKey from core for convenience.
pub use compatibility::PlanRegistryCompatibilityError;
pub use nebula_core::PluginKey;
pub use nebula_metadata::PluginDependency;
pub use nebula_plugin_macros::Plugin;
pub use plan::{
    ActivationDiagnostic, ExecutablePlanIntegrityError, ExecutablePlanRevision,
    PlanBindingContract, PlanBindingRequirement, PlanBindingSite, PlanCompilationError,
    RecordedExecutablePlanRevisionV1,
};
pub use plugin::Plugin;
pub use registry::PluginRegistry;
pub use registry::{FrozenPluginRegistry, RegistryFreezeError};
pub use resolved_plugin::ResolvedPlugin;
