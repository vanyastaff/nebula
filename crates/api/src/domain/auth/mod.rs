//! Authentication domain — Plane A (host / Nebula API sign-in).
//!
//! Per ADR-0033, **Plane A** (who may call this API: identity, sessions,
//! MFA, PATs, user-facing OAuth sign-in) is kept disjoint from **Plane B**
//! (integration credential OAuth — see [`crate::domain::credential`] and
//! [`crate::transport::oauth`]). New auth-domain features land here, never
//! in the credential tree.
//!
//! Self-contained per canon §12.7:
//!
//! - [`routes`] — unauthenticated `/api/v1/auth/*` route table.
//! - [`handler`] — thin HTTP handlers over [`backend::AuthBackend`].
//! - [`backend`] — the Plane-A backend subsystem (trait + in-memory impl +
//!   session / PAT / MFA / password / OAuth primitives + auth DTOs). This is
//!   the production injection point on [`crate::AppState`].

pub mod backend;
pub mod handler;
pub mod routes;
