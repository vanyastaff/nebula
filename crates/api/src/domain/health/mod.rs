//! Health domain — liveness, readiness, version.
//!
//! Self-contained per canon §12.7: route table ([`routes`]), HTTP handlers
//! ([`handler`]), and response DTOs ([`dto`]) live together. Mounted at the
//! root (not under `/api/v1`) by [`crate::domain::create_routes`].

pub mod dto;
pub mod handler;
pub mod routes;
