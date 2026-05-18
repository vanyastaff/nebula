//! Workflow domain — CRUD, activation, and execution.
//!
//! Self-contained per domain-module layout: HTTP handlers ([`handler`]) and
//! request/response DTOs ([`dto`]) live together; the live route table is
//! assembled in [`crate::domain::workspace`] (tenant-prefix nesting) on an
//! `OpenApiRouter` so served paths and the published OpenAPI spec share one
//! source of truth (stub-endpoint policy). integration seam knife seam:
//! [`handler::create_workflow`] / [`handler::activate_workflow`].

pub mod dto;
pub mod handler;
