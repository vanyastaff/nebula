//! # nebula-api
//!
//! HTTP entry point for the Nebula workflow engine (API Gateway pattern).
//! All business logic is delegated to port traits injected into [`AppState`];
//! no SQL drivers or storage schema knowledge live here (canon §12.3).
//!
//! ## Key modules
//!
//! - `domain` — self-contained per-domain modules (canon §12.7), each
//!   `domain/<x>/{routes,handler,dto}.rs`: `auth`, `me`, `org`, `workflow`,
//!   `execution`, `credential`, `catalog`, `health`, `resource`. Route
//!   assembly (`create_routes` + the per-group middleware / tenant nesting)
//!   lives in [`domain`]'s `mod.rs`; the flat `domain::workspace`,
//!   `domain::internal`, and `domain::metrics` modules carry assembly-only
//!   routing. All tenant-scoped routes nest under
//!   `/api/v1/orgs/{org}/workspaces/{ws}/…`. Slug-routed webhooks are
//!   mounted directly by the transport (see [`transport::webhook`]);
//!   internal ops endpoints live under [`domain::internal`].
//! - `domain::shared` — cross-domain DTOs: cursor pagination
//!   (`CursorParams`, `PaginatedResponse<T>`), the page/offset
//!   `PaginationParams`, and the canonical `AckResponse`.
//! - `middleware` — auth (JWT + API-key → `AuthContext`), tenancy (path-based org/workspace
//!   resolution via `nebula-core::Slug`), RBAC, CSRF, tracing, security headers, request ID.
//! - `error` — RFC 9457 `ProblemDetails` / `ApiError`; seam for canon §12.4. Includes
//!   `SessionExpired`, `MfaRequired`, `InsufficientRole`, `OrgNotFound`, `WorkspaceNotFound`,
//!   `SlugConflict`, `CsrfRejected`, `PaginationInvalid`, `RateLimited`, `TenantMismatch` among
//!   others.
//! - `transport::webhook` — converged inbound webhook transport
//!   (programmatic + slug-routed surfaces, M3.3 / ADR-0049):
//!   `WebhookTransport`, `WebhookKey`, `WebhookRateLimiter`,
//!   `EndpointProviderImpl`, storage bootstrap, lifecycle subscriber.
//! - `state` — `AppState` holds port trait references: `WorkflowRepo`, `ExecutionRepo`,
//!   `ControlQueueRepo`, `OrgResolver`, `WorkspaceResolver`, `AuthBackend`, `MembershipStore`.
//! - `config` — `ApiConfig` with sub-configs (`TlsConfig`, `CookieConfig`, `CorsConfig`,
//!   `VersioningConfig`, `PaginationConfig`) / `JwtSecret`; startup fails hard on a missing or
//!   short secret — no `Default` impl (§4.5 operational honesty).
//!
//! ## Authentication planes (ADR-0033)
//!
//! Keep **Plane A** (who may call this API) separate from **Plane B** (integration credentials
//! for workflows talking to *external* systems):
//!
//! - **[`domain::auth`]** + **`middleware::auth`** — **Plane A**: identity, sessions, MFA, PATs,
//!   and the user-facing OAuth sign-in flow plus the cookie / JWT / `X-API-Key` middleware that
//!   gates the Nebula API itself. The Plane-A backend subsystem lives under
//!   [`domain::auth::backend`].
//! - **[`domain::credential`]** — **Plane B infrastructure**: OAuth2 flow helpers (PKCE, signed
//!   state, token exchange) and input validators for integration credentials. Flow helpers live
//!   under [`transport::oauth`] with validators in [`extractors::credential`]. HTTP handlers live
//!   in [`domain::credential::handler`]; route wiring in [`domain::workspace`] and
//!   [`domain::credential::routes`]. All credential routes are **protected by Plane A** middleware.
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
pub mod domain;
pub mod error;
pub mod extractors;
pub mod middleware;
pub mod openapi;
pub mod ports;
pub mod state;
pub mod telemetry_init;
mod trace_capture;
pub mod transport;

pub use app::build_app;
pub use config::{ApiConfig, ApiConfigError, JwtSecret};
pub use domain::resource::handler::map_resource_update_storage_error;
pub use domain::shared::{CursorParams, PaginatedResponse};
pub use error::{ApiError, ApiResult, ProblemDetails};
pub use state::AppState;
pub use telemetry_init::init_api_telemetry;
