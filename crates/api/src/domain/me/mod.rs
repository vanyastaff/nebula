//! "Me" domain — authenticated user profile, orgs, personal access tokens.
//!
//! Self-contained per canon §12.7: route table ([`routes`]), HTTP handlers
//! ([`handler`]), and request/response DTOs ([`dto`]) live together.
//! Authenticated, no tenant scope. Currently stubbed (501) per ADR-0047.

pub mod dto;
pub mod handler;
pub mod routes;
