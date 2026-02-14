//! # Nebula Core
//!
//! Core types and traits for the Nebula workflow engine.
//! This crate provides the fundamental building blocks used by all other Nebula crates.
//!
//! ## Key Components
//!
//! - **Identifiers**: ExecutionId, WorkflowId, NodeId, UserId, TenantId, ProjectId, RoleId, OrganizationId
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
//! let execution_id = ExecutionId::v4();
//! let workflow_id = WorkflowId::v4();
//! let node_id = NodeId::v4();
//!
//! let scope = ScopeLevel::Execution(execution_id);
//! ```

pub mod constants;
pub mod id;
pub mod scope;
pub mod traits;
pub mod types;

// Re-export main types for convenience
pub use constants::*;
pub use id::*;
pub use keys::*;
pub use scope::*;
pub use traits::*;
pub use types::*;

// Re-export common error types
pub use error::*;

mod error;
mod keys;

/// Result type used throughout Nebula
pub type Result<T> = std::result::Result<T, error::CoreError>;

/// Common prelude for Nebula crates
pub mod prelude {
    pub use super::{
        CoreError, CredentialId, ExecutionId, HasContext, Identifiable, InterfaceVersion, NodeId,
        NodeKey, NodeKeyError, OrganizationId, ProjectId, ProjectType, Result, RoleId, RoleScope,
        ScopeLevel, Scoped, TenantId, UserId, UuidParseError, WorkflowId,
    };

    pub use crate::keys::*;
}
