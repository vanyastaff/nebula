//! OAuth2 / credential infrastructure — **Plane B** (ADR-0033) / OAuth ceremony (ADR-0031).
//!
//! This module provides the infrastructure for OAuth2 credential acquisition:
//! PKCE flow helpers, signed state management, HTTP token exchange, and input
//! validation. It does **not** contain HTTP route handlers (those live in
//! [`crate::handlers::credential`]) or route definitions (see
//! [`crate::routes::workspace`] and [`crate::routes::credential`]).
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

pub mod flow;
pub mod http;
pub mod state;
