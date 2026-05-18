//! Per-domain modules + route assembly for the Nebula API.
//!
//! Per domain-module layout each domain is a self-contained module
//! `domain/<x>/{routes,handler,dto}.rs` instead of the old triple-spread
//! across `handlers/` + `routes/` + `models/`. Cross-domain shared DTOs
//! live in [`shared`]; the Plane-A auth backend subsystem lives under
//! [`auth::backend`].
//!
//! ## URL structure (spec 05-api-routing)
//!
//! /health, /ready, /metrics, /version — unauthenticated health/internal
//! /api/v1/auth/* — unauthenticated auth endpoints
//! /api/v1/me/* — authenticated, no tenant scope
//! /api/v1/orgs/{org}/* — authenticated, org-scoped
//! /api/v1/orgs/{org}/workspaces/{ws}/* — authenticated, workspace-scoped
//! /api/v1/hooks/{org}/{ws}/{trigger} — special webhook routing (no standard auth)
//! /api/v1/openapi.json, /api/v1/docs — unauthenticated docs
//!
//! Each domain's route table is a `utoipa_axum::router::OpenApiRouter<AppState>`
//! with `#[utoipa::path]`-derived **relative** paths (e.g. `/auth/signup`);
//! the `/api/v1` prefix is applied via `OpenApiRouter::nest` inside
//! `build_openapi_router` so the published spec and the served route
//! table share one source of truth (stub-endpoint policy drift-detection guarantee).

pub mod auth;
pub mod catalog;
pub mod credential;
pub mod execution;
pub mod health;
pub mod internal;
pub mod me;
pub mod metrics;
pub mod org;
pub mod resource;
pub mod shared;
// `pub mod webhook;` — removed by M3.3 task C3. The slug-routed
// surface is mounted by `transport::webhook::WebhookTransport::router`.
pub mod workflow;
pub mod workspace;

use axum::{Router, middleware};
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::{
    config::ApiConfig,
    middleware::{
        auth::auth_middleware, csrf::csrf_middleware, rbac::rbac_middleware,
        tenancy::tenancy_middleware,
    },
    openapi::OpenApiDoc,
    state::AppState,
};

/// Build the full API router and materialised OpenAPI 3.1 document.
///
/// The returned `Router` is bound to `AppState` and ready to compose with
/// the rest of the middleware stack in [`crate::build_app`]. The returned
/// `OpenApi` is handed to `utoipa_swagger_ui::SwaggerUi` in `build_app`,
/// which serves both `/api/v1/openapi.json` (spec JSON) and
/// `/api/v1/docs/` (self-hosted Swagger UI HTML) as a Tower service.
pub fn create_routes(state: AppState, _config: &ApiConfig) -> (Router, OpenApi) {
    let api_router = build_openapi_router(&state);
    let (router, openapi) = api_router.split_for_parts();
    crate::access::assert_tenant_access_coverage(&openapi)
        .expect("tenant routes must declare access permissions");
    let router = router.with_state(state);
    (router, openapi)
}

fn build_openapi_router(state: &AppState) -> OpenApiRouter<AppState> {
    use utoipa::OpenApi as _;

    // Auth routes — no auth middleware, no tenant scope.
    let auth_routes = auth::routes::router();

    // Webhook routes (webhook activation): mounted by `transport.router()`
    // in `app::build_app` directly. The legacy `routes::webhook`
    // module has been absorbed into `transport::webhook::transport`
    // — slug and programmatic surfaces share `dispatch_inner` there.

    // User routes — auth required, no tenant scope.
    let me_routes = me::routes::router()
        .layer(middleware::from_fn(csrf_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Catalog routes — auth required, no tenant scope.
    let catalog_routes = catalog::routes::router().layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
    ));

    // Tenant-scoped routes — auth + tenancy + RBAC + CSRF.
    let tenant_routes = OpenApiRouter::new()
        .merge(org::routes::router())
        .merge(workspace::router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rbac_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            tenancy_middleware,
        ))
        .layer(middleware::from_fn(csrf_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Credential OAuth callback routes (Plane B — API-owned OAuth flow).
    let credential_routes = credential::routes::router().layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
    ));

    // Compose `/api/v1` group. The OpenAPI spec endpoint
    // (`/api/v1/openapi.json`) and the Swagger UI (`/api/v1/docs/`) are
    // mounted by `utoipa_swagger_ui::SwaggerUi` in `app::build_app`
    // *after* `split_for_parts()` materialises the merged spec — those
    // routes therefore do not appear in `paths` (they serve the spec
    // itself, not application content).
    let api_v1 = OpenApiRouter::new()
        .merge(auth_routes)
        .merge(me_routes)
        .merge(catalog_routes)
        .merge(credential_routes)
        .merge(tenant_routes);

    // Public health/readiness/version + Prometheus scrape — no auth, no
    // tenant scope. Mounted at the root, NOT under `/api/v1`.
    OpenApiRouter::with_openapi(OpenApiDoc::openapi())
        .merge(health::routes::router())
        .merge(metrics::router())
        .nest("/api/v1", api_v1)
}
