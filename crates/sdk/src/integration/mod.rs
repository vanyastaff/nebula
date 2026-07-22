//! Curated contracts for integration authors.
//!
//! Prefer these persona-scoped modules over workspace-crate re-exports when
//! writing integrations. Their paths do not expose Nebula's internal crate
//! topology as part of the supported SDK contract.

pub mod credential;
