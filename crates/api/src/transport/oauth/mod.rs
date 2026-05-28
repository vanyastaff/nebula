//! OAuth2 / credential infrastructure — **Plane B** (auth plane separation) / OAuth ceremony (API-owned OAuth flow).
//!
//! This module provides the infrastructure for OAuth2 credential acquisition:
//! PKCE flow helpers, signed state management, HTTP token exchange, and input
//! validation. It does **not** contain HTTP route handlers (those live in
//! [`crate::domain::credential::handler`]) or route definitions (see
//! [`crate::domain::workspace`] and [`crate::domain::credential::routes`]).
//!
//! # Sub-modules
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | [`flow`] | Authorization URI construction and code exchange helpers |
//! | [`state`] | Signed OAuth state (CSRF) generation and verification |
//! | [`http`] | HTTP client for token endpoint requests |
//!
//! Part of `nebula-api` base deps (rollout window closed 2026-04-24).

pub mod discovery;
pub mod flow;
pub mod http;
pub mod state;
pub mod userinfo;
