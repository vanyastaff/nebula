//! `nebula-metadata` — shared metadata shapes for catalog-leaf entities
//! (actions, credentials, resources). See the crate README below for the
//! full surface, composition example, consumer list, and the
//! plugin-as-container carve-out (ADR-0018).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

/// Core [`BaseMetadata`] struct and [`Metadata`] trait.
pub mod base;
/// [`BaseCompatError`] + [`validate_base_compat`] — generic compat rules
/// shared by every catalog citizen.
pub mod compat;
/// [`DeprecationNotice`] — standard deprecation payload.
pub mod deprecation;
/// [`Icon`] enum — one valid representation for catalog icons.
pub mod icon;
/// [`PluginManifest`] — bundle descriptor for a plugin, and [`ManifestError`] for
/// construction failures (ADR-0018, moved from `nebula-plugin` in slice B).
pub mod manifest;
/// [`MaturityLevel`] — `Experimental / Beta / Stable / Deprecated`.
pub mod maturity;

pub use base::{BaseMetadata, Metadata};
pub use compat::{BaseCompatError, validate_base_compat};
pub use deprecation::DeprecationNotice;
pub use icon::Icon;
pub use manifest::{ManifestError, PluginManifest, PluginManifestBuilder, normalize_key};
pub use maturity::MaturityLevel;
