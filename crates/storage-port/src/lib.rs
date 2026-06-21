//! # nebula-storage-port — the storage port
//!
//! Object-safe repository traits, port-local DTOs, the plain-data [`Scope`]
//! value type, and the [`TransitionBatch`] atomic unit-of-work. No backend
//! code lives here.
//!
//! The port defines the contract every storage backend (in-memory, SQLite,
//! Postgres) must satisfy. Consumers (engine, api, core) depend only on this
//! crate so they stay testable without a database driver. The plain-data
//! [`Scope`] value type lives here so tenant-scoped signatures can require it
//! without an upward dependency on the tenancy policy crate.
#![warn(missing_docs)]
#![warn(clippy::all)]

mod batch;
/// Port-local row/record DTOs.
pub mod dto;
mod error;
/// Id seam: re-exported `nebula-core` identifiers + the lease
/// [`ids::FencingToken`]. The port reuses core's typed ULIDs verbatim
/// rather than re-defining them.
pub mod ids;
mod scope;
/// Repository traits (ISP-segregated, object-safe).
pub mod store;

pub use batch::{TransitionBatch, TransitionBatchBuilder, TransitionOutcome};
pub use dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash, TokenHashLengthError};
pub use error::StorageError;
pub use ids::FencingToken;
pub use scope::Scope;
