//! # nebula-api
//!
//! HTTP entry point for the Nebula workflow engine (API Gateway pattern).
//! All business logic is delegated to port traits injected into [`AppState`];
//! no SQL drivers or storage schema knowledge live here (canon §12.3).
//!
//! ## Key modules
//!
//! - `handlers` — thin HTTP handlers: extract, validate, delegate, return. Includes `auth`, `me`,
//!   `org`, `workflow`, `execution`, `credential`, `catalog`, `openapi`, `webhook`.
//! - `middleware` — auth (JWT + API-key → `AuthContext`), tenancy (path-based org/workspace
//!   resolution via `nebula-core::Slug`), RBAC, CSRF, tracing, security headers, request ID.
//! - `errors` — RFC 9457 `ProblemDetails` / `ApiError`; seam for canon §12.4. Includes
//!   `SessionExpired`, `MfaRequired`, `InsufficientRole`, `OrgNotFound`, `WorkspaceNotFound`,
//!   `SlugConflict`, `CsrfRejected`, `PaginationInvalid`, `RateLimited`, `TenantMismatch` among
//!   others.
//! - `pagination` — cursor-based pagination: `CursorParams`, `PaginatedResponse<T>`.
//! - `webhook` — inbound trigger transport: `WebhookTransport` activate/deactivate/router,
//!   `EndpointProviderImpl`, `WebhookRateLimiter` (§11.3 / §13.4). Located under
//!   [`services::webhook`] with rate limiting in [`middleware::webhook_ratelimit`].
//! - `state` — `AppState` holds port trait references: `WorkflowRepo`, `ExecutionRepo`,
//!   `ControlQueueRepo`, `OrgResolver`, `WorkspaceResolver`, `SessionStore`, `MembershipStore`.
//! - `config` — `ApiConfig` with sub-configs (`TlsConfig`, `CookieConfig`, `CorsConfig`,
//!   `VersioningConfig`, `PaginationConfig`) / `JwtSecret`; startup fails hard on a missing or
//!   short secret — no `Default` impl (§4.5 operational honesty).
//! - `routes` — domain-scoped route builders: `auth`, `me`, `org`, `workspace`, `workflow`,
//!   `execution`, `credential`, `catalog`, `webhook`, `openapi`. All tenant-scoped routes nest
//!   under `/api/v1/orgs/{org}/workspaces/{ws}/…`.
//!
//! ## Authentication planes (ADR-0033)
//!
//! Keep **Plane A** (who may call this API) separate from **Plane B** (integration credentials
//! for workflows talking to *external* systems):
//!
//! - **`middleware::auth`** — **Plane A**: JWT bearer and `X-API-Key` for the Nebula API itself.
//! - **`credential`** — **Plane B infrastructure**: OAuth2 flow
//!   helpers (PKCE, signed state, token exchange) and input validators for integration credentials.
//!   Located under [`services::oauth`] with validators in [`extractors::credential`]. HTTP handlers
//!   live in [`handlers::credential`]; route wiring in [`routes::workspace`] and
//!   [`routes::credential`]. All credential routes are **protected by Plane A** middleware.
//!
//! Do not merge these into one conceptual “auth” module — naming stays explicit per ADR-0033.
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
pub mod pagination;
pub mod routes;
pub mod server;
pub mod services;
pub mod state;

pub use app::build_app;
pub use config::{ApiConfig, ApiConfigError, JwtSecret};
pub use errors::{ApiError, ApiResult};
pub use pagination::{CursorParams, PaginatedResponse};
pub use state::AppState;
