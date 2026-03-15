//! # Nebula Core
//!
//! Core types and traits for the Nebula workflow engine.
//! This crate provides the fundamental building blocks used by all other Nebula crates.
//!
//! ## Key Components
//!
//! - **Identifiers**: UserId, TenantId, ExecutionId, WorkflowId, NodeId, ResourceId, CredentialId, ProjectId, RoleId, OrganizationId.  
//!   (Node = workflow step / graph vertex; which action/plugin runs there is given by [`ActionKey`] = [`PluginKey`]; [`NodeDefinition`](https://docs.rs/nebula-workflow) has `action_key: ActionKey`.)
//! - **Keys**: PluginKey (plugin type, e.g. `telegram_bot`), ActionKey (action within a plugin, e.g. `send_message`), ParameterKey, CredentialKey.
//! - **Scope System**: Resource lifecycle management with different scope levels (Global, Organization, Project, Workflow, Execution, Action)
//! - **Base Traits**: Scoped, HasContext, Identifiable for common functionality
//! - **Common Types**: Utilities and constants used throughout the system
//! - **Multi-tenancy Types**: ProjectType, RoleScope for identity and access management
//!
//! ## Usage
//!
//! ```rust
//! use nebula_core::{
//!     ExecutionId, WorkflowId, NodeId,
//!     ScopeLevel, Scoped, HasContext
//! };
//!
//! let execution_id = ExecutionId::new();
//! let workflow_id = WorkflowId::new();
//! let node_id = NodeId::new();
//!
//! let scope = ScopeLevel::Execution(execution_id);
//! ```

pub mod constants;
/// Dependency graph primitives shared across crates.
pub mod deps;
pub mod id;
pub mod scope;
/// Shared serde helpers (duration serialization, etc.).
pub mod serde_helpers;
pub mod traits;
pub mod types;

// Re-export main types for convenience at the crate root. Downstream crates
// should prefer `nebula_core::prelude::*` for a stable import surface.
pub use constants::*;
pub use deps::*;
pub use error::*;
pub use id::*;
pub use keys::*;
pub use scope::*;
pub use traits::*;
pub use types::*;

mod error;
mod keys;

/// Result type used throughout Nebula
pub type Result<T> = std::result::Result<T, error::CoreError>;

/// Common prelude for Nebula crates
pub mod prelude {
    // Identifiers (UUID-backed ids)
    pub use crate::id::{
        CredentialId, ExecutionId, NodeId, OrganizationId, ProjectId, ResourceId, RoleId, TenantId,
        UserId, WorkflowId,
    };

    // Domain keys (normalized string keys)
    pub use crate::keys::{ActionKey, CredentialKey, ParameterKey, PluginKey, ResourceKey};

    // Core errors and parse errors
    pub use crate::error::CoreError;
    pub use crate::scope::{ScopeLevel, ScopeResolver};
    pub use crate::traits::{HasContext, Scoped};
    pub use crate::types::{InterfaceVersion, ProjectType, RoleScope};
    pub use domain_key::{KeyParseError, UuidParseError};

    // Dependency error type
    pub use crate::deps::DependencyError;

    // Core result alias
    pub use crate::Result;
}
