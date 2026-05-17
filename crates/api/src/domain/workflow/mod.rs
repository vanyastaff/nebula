//! Workflow domain — CRUD, activation, and execution.
//!
//! Self-contained per canon §12.7: HTTP handlers ([`handler`]) and
//! request/response DTOs ([`dto`]) live together; the live route table is
//! assembled in [`crate::domain::workspace`] (tenant-prefix nesting) on an
//! `OpenApiRouter` so served paths and the published OpenAPI spec share one
//! source of truth (ADR-0047). §13 knife seam:
//! [`handler::create_workflow`] / [`handler::activate_workflow`].

pub mod dto;
pub mod handler;
