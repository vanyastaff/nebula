//! Catalog domain — action and plugin discovery.
//!
//! Self-contained per domain-module layout: route table ([`routes`]), HTTP handlers
//! ([`handler`]), and response DTOs ([`dto`]) live together.

pub mod dto;
pub mod handler;
pub mod routes;
