//! # Nebula Core
//!
//! Core types and traits for the Nebula workflow engine.
//! This crate provides the fundamental building blocks used by all other Nebula crates.
//!
//! ## Key Components
//!
//! - **Identifiers**: ExecutionId, WorkflowId, NodeId, UserId, TenantId
//! - **Scope System**: Resource lifecycle management with different scope levels
//! - **Base Traits**: Scoped, HasContext, Identifiable for common functionality
//! - **Common Types**: Utilities and constants used throughout the system
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
//! let workflow_id = WorkflowId::new("my-workflow");
//! let node_id = NodeId::new("process-data");
//!
//! let scope = ScopeLevel::Execution(execution_id.clone());
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
        CoreError, CredentialId, ExecutionId, HasContext, Identifiable, NodeId, Result, ScopeLevel,
        Scoped, TenantId, UserId, WorkflowId,
    };

    pub use crate::keys::*;
}
