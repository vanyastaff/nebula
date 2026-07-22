//! Credential domain — Plane B (integration credentials for workflows
//! talking to *external* systems; auth plane separation).
//!
//! Kept disjoint from Plane A ([`crate::domain::auth`]) per auth plane separation. All
//! credential routes are protected by Plane A middleware.
//!
//! Self-contained per domain-module layout:
//!
//! - [`routes`] — system-level credential route table (type discovery).
//!   Workspace-scoped credential CRUD and universal acquisition are merged by
//!   [`crate::domain::workspace`].
//! - [`handler`] — workspace-scoped CRUD, lifecycle, universal acquisition,
//!   and type discovery handlers.
//! - [`dto`] — request/response shapes for credential endpoints.

pub mod dto;
pub mod handler;
pub mod routes;
pub mod schema_projection;
