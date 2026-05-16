//! "Me" domain — authenticated user profile, orgs, personal access tokens.
//!
//! Self-contained per canon §12.7: route table ([`routes`]), HTTP handlers
//! ([`handler`]), and request/response DTOs ([`dto`]) live together.
//! Authenticated, no tenant scope. Profile, PAT, and `list_my_orgs`
//! endpoints are real end-to-end via the Plane-A `AuthBackend` port and
//! the `MembershipStore` org-membership backing (Phase 3 "Option 1",
//! canon §4.5).

pub mod dto;
pub mod handler;
pub mod routes;
