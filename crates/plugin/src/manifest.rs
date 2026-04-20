//! `PluginManifest` is canonical in `nebula-metadata` (ADR-0018 follow-up,
//! moved there in slice B of the plugin load-path stabilization). This module
//! re-exports the type for source compatibility of callers that still write
//! `use nebula_plugin::PluginManifest;`.
//!
//! New code should import directly from `nebula_metadata::PluginManifest`.
//!
//! See [`nebula_metadata::manifest`] for the canonical definition.

pub use nebula_metadata::{ManifestError, PluginManifest, PluginManifestBuilder};
