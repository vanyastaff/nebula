//! `nebula-metadata` — shared metadata shapes for named, schematized entities.
//!
//! Every "catalog citizen" in Nebula — an action, a credential, a resource,
//! a trigger, a future plugin — shares the same surface:
//!
//! - a typed [`key`](Metadata::key) that identifies it,
//! - a human-readable [`name`](Metadata::name) and [`description`](Metadata::description),
//! - a canonical [`ValidSchema`](nebula_schema::ValidSchema) describing its user-configurable
//!   inputs,
//! - optional catalog ornaments — [`icon`](Metadata::icon), documentation URL,
//!   [`tags`](Metadata::tags),
//! - a [`MaturityLevel`] and optional [`DeprecationNotice`].
//!
//! This crate owns those shared concerns as concrete types and a small trait,
//! so each business-layer crate (action, credential, resource, …) composes
//! them instead of redeclaring the same prefix with incompatible field names.
//!
//! # Shape
//!
//! ```no_run
//! use nebula_metadata::{BaseMetadata, Icon, MaturityLevel, Metadata};
//!
//! pub struct MyKey;
//!
//! pub struct MyEntityMetadata {
//!     pub base: BaseMetadata<MyKey>,
//!     pub extra_field: u32,
//! }
//!
//! impl Metadata for MyEntityMetadata {
//!     type Key = MyKey;
//!     fn base(&self) -> &BaseMetadata<Self::Key> {
//!         &self.base
//!     }
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Core [`BaseMetadata`] struct and [`Metadata`] trait.
pub mod base;
/// [`BaseCompatError`] + [`validate_base_compat`] — generic compat rules
/// shared by every catalog citizen.
pub mod compat;
/// [`DeprecationNotice`] — standard deprecation payload.
pub mod deprecation;
/// [`Icon`] enum — one valid representation for catalog icons.
pub mod icon;
/// [`MaturityLevel`] — `Experimental / Beta / Stable / Deprecated`.
pub mod maturity;

pub use base::{BaseMetadata, Metadata};
pub use compat::{BaseCompatError, validate_base_compat};
pub use deprecation::DeprecationNotice;
pub use icon::Icon;
pub use maturity::MaturityLevel;
