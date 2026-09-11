//! Shared metadata authoring, admission, recorded evidence, access, and
//! compatibility for catalog-leaf entities. See the crate README below for the
//! complete construction and restoration model.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

/// Draft, admitted, and recorded base metadata plus the [`Metadata`] trait.
pub mod base;
/// [`BaseCompatError`] + [`validate_base_compat`] — generic compat rules
/// shared by every catalog citizen.
pub mod compat;
/// Shared `serde` default/`skip_serializing_if` helpers used by both
/// [`base`] and [`manifest`] — not part of the public API.
mod defaults;
mod definition;
/// [`DeprecationNotice`] — standard deprecation payload.
pub mod deprecation;
/// [`Icon`] enum — one valid representation for catalog icons.
pub mod icon;
/// [`PluginManifest`] — bundle descriptor for a plugin, and [`ManifestError`] for
/// construction failures.
pub mod manifest;
/// [`MaturityLevel`] — `Experimental / Beta / Stable / Deprecated`.
pub mod maturity;
mod name;

pub use base::{BaseMetadata, Metadata, MetadataDraft, RecordedBaseMetadata};
pub use compat::{BaseCompatError, validate_base_compat};
pub use definition::{MetadataBuildError, MetadataError, MetadataReadmissionError};
pub use deprecation::DeprecationNotice;
pub use icon::Icon;
pub use manifest::{ManifestError, PluginDependency, PluginManifest, PluginManifestBuilder};
pub use maturity::MaturityLevel;
pub use name::MetadataName;
#[doc(hidden)]
pub use name::MetadataNameLiteral;

/// Semantic version used by catalog metadata.
pub type MetadataVersion = semver::Version;
