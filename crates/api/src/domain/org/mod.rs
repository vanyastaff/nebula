//! Organization domain — org settings, members, service accounts.
//!
//! Self-contained per canon §12.7: route table ([`routes`]), HTTP handlers
//! ([`handler`]), and request/response DTOs ([`dto`]) live together.
//! Authenticated + org-scoped. Currently stubbed (501) per ADR-0047; the
//! RBAC `tenant.require(...)` gates are real today.

pub mod dto;
pub mod handler;
pub mod routes;
