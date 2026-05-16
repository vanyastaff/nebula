//! Catalog domain — action and plugin discovery.
//!
//! Self-contained per canon §12.7: route table ([`routes`]), HTTP handlers
//! ([`handler`]), and response DTOs ([`dto`]) live together.

pub mod dto;
pub mod handler;
pub mod routes;
