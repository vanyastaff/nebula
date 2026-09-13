//! Lossless schema graph documents and checked admission.
//!
//! Wire v3 keeps executable schema semantics separate from presentation and
//! admits the document before exposing commitments or resolved addresses.
//!
//! ```rust
//! use nebula_schema::SchemaGraphDocument;
//! use serde_json::json;
//!
//! let document: SchemaGraphDocument = serde_json::from_value(json!({
//!     "version": 3,
//!     "root": { "target": "text", "null": "reject" },
//!     "definitions": [{ "key": "text", "body": { "kind": "string" } }],
//!     "x-vendor": { "retained": true }
//! }))?;
//! let graph = document.admit()?;
//! assert_eq!(graph.reference_count(), 1);
//! assert_eq!(graph.semantic_commitment().as_bytes().len(), 32);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod admission;
mod canonical;
mod document;
mod error;
mod lower;
mod model;
mod number;
mod rule_canonical;
mod view;

pub use admission::{
    AdmittedDeclarationAddress, AdmittedSchemaGraph, DeclarationAddress, DeclarationUse,
    DefinitionMemberKey,
};
pub use canonical::{AddressSpaceCommitment, SemanticCommitment};
pub use document::SchemaGraphDocument;
pub use error::SchemaAdmissionError;
pub use model::DefinitionKey;

/// Current semantic schema graph wire version.
pub const SCHEMA_GRAPH_WIRE_VERSION: u16 = 3;

/// Maximum definitions admitted in one graph.
pub const MAX_GRAPH_DEFINITIONS: usize = 1_024;

/// Maximum reference edges admitted in one graph.
pub const MAX_GRAPH_REFERENCES: usize = 4_096;

/// Maximum aggregate UTF-8 bytes in graph identifiers.
pub const MAX_GRAPH_IDENTIFIER_BYTES: usize = 128 * 1_024;

/// Maximum bytes retained by a lossless graph document.
pub const MAX_GRAPH_DOCUMENT_BYTES: usize = 1024 * 1024;

/// Maximum bytes in either private canonical commitment encoding.
pub const MAX_GRAPH_CANONICAL_BYTES: usize = 1024 * 1024;

/// Maximum diagnostics returned by one admission attempt.
pub const MAX_GRAPH_DIAGNOSTICS: usize = 128;
