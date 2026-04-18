//! # nebula-api
//!
//! HTTP entry point for the Nebula workflow engine (API Gateway pattern).
//! All business logic is delegated to port traits injected into [`AppState`];
//! no SQL drivers or storage schema knowledge live here (canon §12.3).
//!
//! ## Key modules
//!
//! - `handlers` — thin HTTP handlers: extract, validate, delegate, return.
//! - `middleware` — auth (JWT + API-key), tracing, security headers, request ID.
//! - `errors` — RFC 9457 `ProblemDetails` / `ApiError`; seam for canon §12.4.
//! - `webhook` — inbound trigger transport: `WebhookTransport` activate/deactivate/router,
//!   `EndpointProviderImpl`, `WebhookRateLimiter` (§11.3 / §13.4).
//! - `state` — `AppState` holds port trait references only; no concrete impls.
//! - `config` — `ApiConfig` / `JwtSecret`; startup fails hard on a missing or short secret — no
//!   `Default` impl (§4.5 operational honesty).
//!
//! ## Canon invariants
//!
//! - **§12.4** — All errors are RFC 9457 `application/problem+json`; no new ad-hoc 500s for
//!   business-logic failures.
//! - **§13 steps 1–3, 5** — This crate is the entry point for the knife scenario: create → activate
//!   → start → … → cancel.
//! - **§12.3** — Local-first: starts with in-memory repos; no Docker required.
//! - **§4.5** — No capability is advertised that the engine does not honor end-to-end.

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod app;
pub mod config;
pub mod errors;
pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod routes;
pub mod services;
pub mod state;
pub mod webhook;

pub use app::build_app;
pub use config::{ApiConfig, ApiConfigError, JwtSecret};
pub use state::AppState;
