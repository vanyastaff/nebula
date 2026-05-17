//! Credential domain — Plane B (integration credentials for workflows
//! talking to *external* systems; ADR-0031 / ADR-0033).
//!
//! Kept disjoint from Plane A ([`crate::domain::auth`]) per ADR-0033. All
//! credential routes are protected by Plane A middleware.
//!
//! Self-contained per canon §12.7:
//!
//! - [`routes`] — system-level credential route table (type discovery +
//!   OAuth2 callbacks). Workspace-scoped credential CRUD is merged by
//!   [`crate::domain::workspace`].
//! - [`handler`] — workspace-scoped CRUD, lifecycle, acquisition, type
//!   discovery, and OAuth2 transport HTTP handlers.
//! - [`oauth`] — OAuth2 controller primitives (PKCE, signed state, token
//!   exchange) the handlers delegate to; OAuth flow helpers proper live in
//!   [`crate::transport::oauth`].
//! - [`dto`] — request/response shapes for credential endpoints.

pub mod dto;
pub mod handler;
pub mod oauth;
pub mod routes;
pub mod schema_projection;
