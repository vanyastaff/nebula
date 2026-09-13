//! Curated contracts for integration authors.
//!
//! Prefer these persona-scoped modules over workspace-crate re-exports when
//! writing integrations. Their paths do not expose Nebula's internal crate
//! topology as part of the supported SDK contract.

pub mod action;
pub mod credential;
pub mod resource;

pub use nebula_metadata::{
    CatalogCategoryKey, CatalogLink, CatalogLinkRelation, CatalogLinkTarget, CatalogReference,
    CatalogValueError, DeprecationNotice, DocumentationOrigin, Icon, MaturityLevel, MetadataError,
    MetadataField, MetadataName, MetadataVersion, RemovalDate, RemovalMilestone, RemovalSchedule,
    metadata_name,
};
pub use semver::VersionReq;
