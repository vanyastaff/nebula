//! Concrete repo backends.
//!
//! Each module implements the layer-1 repo traits ([`ExecutionRepo`],
//! [`WorkflowRepo`]) for a specific persistence backend. The legacy generic
//! `Storage` key/value trait was retired in the audit P1 sweep — concrete
//! repos are the canonical surface.
//!
//! [`ExecutionRepo`]: crate::ExecutionRepo
//! [`WorkflowRepo`]: crate::WorkflowRepo

#[cfg(feature = "postgres")]
mod pg_execution;
#[cfg(feature = "postgres")]
mod postgres;

#[cfg(feature = "postgres")]
pub use pg_execution::PgExecutionRepo;
#[cfg(feature = "postgres")]
pub use postgres::{PgWorkflowRepo, PostgresStorage, PostgresStorageConfig};
