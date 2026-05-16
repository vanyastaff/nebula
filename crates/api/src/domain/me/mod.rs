//! "Me" domain — authenticated user profile, orgs, personal access tokens.
//!
//! Self-contained per canon §12.7: route table ([`routes`]), HTTP handlers
//! ([`handler`]), and request/response DTOs ([`dto`]) live together.
//! Authenticated, no tenant scope. Profile + PAT endpoints are real
//! end-to-end via the Plane-A `AuthBackend` port; `list_my_orgs` remains
//! an honest 501 stub until the org/membership phase (canon §4.5).

pub mod dto;
pub mod handler;
pub mod routes;
