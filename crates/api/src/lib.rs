//! # nebula-api
//!
//! HTTP entry point for the Nebula workflow engine (API Gateway pattern).
//! All business logic is delegated to port traits injected into [`AppState`];
//! no SQL drivers or storage schema knowledge live here (API purity boundary).
//!
//! ## Key modules
//!
//! - `domain` — self-contained per-domain modules (domain-module layout), each
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
//! - `error` — RFC 9457 `ProblemDetails` / `ApiError`; seam for problem+json error seam. Includes
//!   `SessionExpired`, `MfaRequired`, `InsufficientRole`, `OrgNotFound`, `WorkspaceNotFound`,
//!   `SlugConflict`, `CsrfRejected`, `PaginationInvalid`, `RateLimited`, `TenantMismatch` among
//!   others.
//! - `transport::webhook` — converged inbound webhook transport
//!   (programmatic + slug-routed surfaces, webhook activation):
//!   `WebhookTransport`, `WebhookKey`, `WebhookRateLimiter`,
//!   `EndpointProviderImpl`, storage bootstrap, lifecycle subscriber.
//! - `state` — `AppState` holds port trait references: `WorkflowRepo`, `ExecutionRepo`,
//!   `ControlQueueRepo`, `OrgResolver`, `WorkspaceResolver`, `AuthBackend`, `MembershipStore`.
//! - `config` — `ApiConfig` with sub-configs (`TlsConfig`, `CookieConfig`, `CorsConfig`,
//!   `VersioningConfig`, `PaginationConfig`) / `JwtSecret`; startup fails hard on a missing or
//!   short secret — no `Default` impl (honest capability operational honesty).
//!
//! ## Authentication planes (auth plane separation)
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
//! Do not merge these into one conceptual “auth” module — naming stays explicit per auth plane separation.
//!
//! ## Canon invariants
//!
//! - **problem+json** — All errors are RFC 9457 `application/problem+json`; no new ad-hoc 500s for
//!   business-logic failures.
//! - **integration seam steps 1–3, 5** — This crate is the entry point for the knife scenario: create → activate
//!   → start → … → cancel.
//! - **API purity** — Local-first: starts with in-memory repos; no Docker required.
//! - **honest capability** — No capability is advertised that the engine does not honor end-to-end.

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod access;
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

/// Test-only helpers for `nebula-api` integration tests against
/// localhost wiremock IdPs. Gated by the custom cfg `nebula_test_util`
/// (NOT a Cargo feature — features are additive and can transitively
/// activate; custom cfg requires explicit `RUSTFLAGS="--cfg
/// nebula_test_util"` opt-in per the tokio_unstable precedent).
///
/// Tests enable via:
/// ```sh
/// RUSTFLAGS="--cfg nebula_test_util" cargo nextest run -p nebula-api --test oauth_provider_e2e
/// ```
///
/// See ADR-0085 D-14 and PR-2 tasks T2.11 / T2.12.
#[cfg(nebula_test_util)]
pub mod test_support;

// Release-build guard per ADR-0085 D-14 + tasks T2.12: if the
// `nebula_test_util` cfg is somehow set in a release build (operator
// passes `RUSTFLAGS="--cfg nebula_test_util"` to `cargo build
// --release`), refuse to compile. `not(debug_assertions)` is the
// canonical release-profile detection cfg (set automatically by
// `cargo build --release` and any `[profile.<name>] debug-assertions =
// false`). The earlier proposal `cfg(feature = "release")` was
// structurally wrong — release is a Cargo profile, NOT a feature.
#[cfg(all(nebula_test_util, not(debug_assertions)))]
compile_error!(
    "nebula_test_util cfg must NOT be active in release builds; \
     remove --cfg nebula_test_util from RUSTFLAGS. \
     See ADR-0085 D-14 for the test-only bypass-helpers contract."
);

pub use app::build_app;
pub use config::{ApiConfig, ApiConfigError, JwtSecret};
pub use domain::resource::handler::{
    map_resource_create_storage_error, map_resource_update_storage_error,
};
pub use domain::shared::{CursorParams, PaginatedResponse};
pub use error::{ApiError, ApiResult, ProblemDetails};
pub use state::AppState;
pub use telemetry_init::{TelemetryGuard, TelemetryInitError, init_api_telemetry};
