//! Resource domain — workspace resource listing.
//!
//! Self-contained per canon §12.7: HTTP handler ([`handler`]) and response
//! DTOs ([`dto`]) live together; the live route table is assembled in
//! [`crate::domain::workspace`] (tenant-prefix nesting) on an
//! `OpenApiRouter` so served paths and the published OpenAPI spec share one
//! source of truth (ADR-0047). The single endpoint is tenant-scoped and
//! currently a 501 stub per ADR-0047.

pub mod dto;
pub mod handler;
