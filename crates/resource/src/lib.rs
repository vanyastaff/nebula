//! # nebula-resource
//!
//! Type-safe, topology-agnostic resource management for the Nebula workflow
//! engine.
//!
//! This crate provides the foundational primitives for managing external
//! resources (databases, HTTP clients, message brokers, etc.) with a unified
//! lifecycle: create, health-check, shutdown, destroy.
//!
//! ## Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Resource`] | Core trait — 5 associated types, 4 lifecycle methods |
//! | [`ResourceHandle`] | RAII lease handle with Owned/Guarded/Shared modes |
//! | [`Cell`] | Lock-free `ArcSwap`-based cell for resident topologies |
//! | [`ReleaseQueue`] | Background worker pool for async cleanup |
//! | [`Error`] | Unified error with [`ErrorKind`] + [`ErrorScope`] |
//! | [`Ctx`] | Execution context with cancellation and extensions |

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod cell;
#[allow(deprecated)]
pub mod compat;
pub mod ctx;
pub mod error;
pub mod handle;
pub mod manager;
pub mod options;
pub mod release_queue;
pub mod resource;
pub mod state;

pub use cell::Cell;
pub use ctx::{BasicCtx, Ctx, Extensions, ScopeLevel, ctx_ext};
pub use error::{Error, ErrorKind, ErrorScope};
pub use handle::ResourceHandle;
pub use manager::Manager;
pub use options::{AcquireIntent, AcquireOptions};
pub use release_queue::ReleaseQueue;
pub use resource::{AnyResource, Credential, Resource, ResourceConfig, ResourceMetadata};
pub use state::{ResourcePhase, ResourceStatus};

pub use nebula_core::{ExecutionId, ResourceKey, WorkflowId};

// Backward-compatibility re-exports (deprecated, will be removed).
#[allow(deprecated)]
pub use compat::{Context, Scope};
