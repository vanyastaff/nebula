//! Shared metadata authoring, admission, recorded evidence, access, and
//! compatibility for catalog-leaf entities. See the crate README below for the
//! complete construction and restoration model.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

/// Draft, admitted, and recorded base metadata plus the [`Metadata`] trait.
pub mod base;
mod bounded;
mod catalog_error;
mod category;
/// [`BaseCompatError`] + [`validate_base_compat`] — generic compat rules
/// shared by every catalog citizen.
pub mod compat;
mod decode;
/// Shared `serde` default/`skip_serializing_if` helpers used by both
/// [`base`] and [`manifest`] — not part of the public API.
mod defaults;
mod definition;
/// [`DeprecationNotice`] — standard deprecation payload.
pub mod deprecation;
/// [`Icon`] enum — one valid representation for catalog icons.
pub mod icon;
mod link;
/// [`PluginManifest`] — bundle descriptor for a plugin, and [`ManifestError`] for
/// construction failures.
pub mod manifest;
/// [`MaturityLevel`] — `Experimental / Beta / Stable / Deprecated`.
pub mod maturity;
mod name;
mod reference;
mod removal;
mod shared;

pub use base::{BaseMetadata, Metadata, MetadataDraft, RecordedBaseMetadata};
pub use catalog_error::CatalogValueError;
pub use category::CatalogCategoryKey;
pub use compat::{BaseCompatError, validate_base_compat};
pub use decode::{
    MAX_METADATA_JSON_BYTES, MAX_METADATA_SCHEMA_BYTES, MAX_SHARED_METADATA_BYTES,
    METADATA_WIRE_VERSION, MetadataDecodeError, MetadataDecodeLimits, check_json_record,
    decode_json_reader, decode_json_slice, deserialize_metadata_object,
};
pub use definition::{MetadataBuildError, MetadataError, MetadataField, MetadataReadmissionError};
pub use deprecation::DeprecationNotice;
pub use icon::Icon;
pub use link::{CatalogLink, CatalogLinkRelation, CatalogLinkTarget, DocumentationOrigin};
pub use manifest::{ManifestError, PluginDependency, PluginManifest, PluginManifestBuilder};
pub use maturity::MaturityLevel;
pub use name::MetadataName;
#[doc(hidden)]
pub use name::MetadataNameLiteral;
pub use reference::CatalogReference;
pub use removal::{RemovalDate, RemovalMilestone, RemovalSchedule};

/// Semantic version used by catalog metadata.
pub type MetadataVersion = semver::Version;
