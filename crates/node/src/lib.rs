//! # Nebula Node
//!
//! Node type system for the Nebula workflow engine.
//!
//! A **node** is the user-visible, versionable unit in Nebula — for example
//! "Slack", "HTTP Request", or "PostgreSQL". Each node bundles:
//!
//! - [`NodeMetadata`] — key, name, version, group, icon, docs URL
//! - Parameter schemas ([`ParameterCollection`])
//! - Credential requirements ([`CredentialDescription`])
//! - Action references (action keys that the engine resolves to [`ActionHandler`]s)
//!
//! ## Core Types
//!
//! - [`Node`] — base trait every node implements
//! - [`NodeMetadata`] — static descriptor with builder API
//! - [`NodeType`] — enum wrapping a single node or a versioned set
//! - [`NodeVersions`] — multi-version container keyed by `u32`
//! - [`NodeRegistry`] — in-memory registry mapping [`NodeKey`] → [`NodeType`]
//! - [`NodeError`] — error type for node operations
//!
//! ## Dynamic Loading (feature-gated)
//!
//! With the `dynamic-loading` feature enabled, [`NodeLoader`] can load
//! node plugins from shared libraries (`.dll` / `.so` / `.dylib`).

// `deny` instead of `forbid` so the `loader` module can use `allow(unsafe_code)` for FFI.
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod error;
#[cfg(feature = "dynamic-loading")]
mod loader;
mod metadata;
mod node;
mod node_type;
mod registry;
mod versions;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use error::NodeError;
#[cfg(feature = "dynamic-loading")]
pub use loader::{NodeLoadError, NodeLoader};
pub use metadata::NodeMetadata;
pub use node::Node;
pub use node_type::NodeType;
pub use registry::NodeRegistry;
pub use versions::NodeVersions;

// Re-export NodeKey from core for convenience.
pub use nebula_core::NodeKey;
