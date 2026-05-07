//! Route assembly for the Nebula API.
//!
//! URL structure per spec 05-api-routing:
//!
//! /health, /ready, /metrics, /version — unauthenticated health/internal
//! /api/v1/auth/* — unauthenticated auth endpoints
//! /api/v1/me/* — authenticated, no tenant scope
//! /api/v1/orgs/{org}/* — authenticated, org-scoped
//! /api/v1/orgs/{org}/workspaces/{ws}/* — authenticated, workspace-scoped
//! /api/v1/hooks/{org}/{ws}/{trigger} — special webhook routing (no standard auth)
//! /api/v1/openapi.json, /api/v1/docs — unauthenticated docs
//!
//! Each sub-router is now a `utoipa_axum::router::OpenApiRouter<AppState>`
//! with `#[utoipa::path]`-derived **relative** paths (e.g. `/auth/signup`);
//! the `/api/v1` prefix is applied via `OpenApiRouter::nest` inside
//! `build_openapi_router` so the published spec and the served route
//! table share one source of truth (ADR-0047 drift-detection guarantee).

pub mod auth;
pub mod catalog;
pub mod health;
pub mod me;
pub mod metrics;
pub mod org;
// `pub mod webhook;` — removed by M3.3 task C3. The slug-routed
// surface is mounted by `services::webhook::WebhookTransport::router`.
pub mod workspace;

pub mod credential;

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
    let router = router.with_state(state);
    (router, openapi)
}

fn build_openapi_router(state: &AppState) -> OpenApiRouter<AppState> {
    use utoipa::OpenApi as _;

    // Auth routes — no auth middleware, no tenant scope.
    let auth_routes = auth::router();

    // Webhook routes (M3.3 / ADR-0049): mounted by `transport.router()`
    // in `app::build_app` directly. The legacy `routes::webhook`
    // module has been absorbed into `services::webhook::transport`
    // — slug and programmatic surfaces share `dispatch_inner` there.

    // User routes — auth required, no tenant scope.
    let me_routes = me::router()
        .layer(middleware::from_fn(csrf_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Catalog routes — auth required, no tenant scope.
    let catalog_routes = catalog::router().layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
    ));

    // Tenant-scoped routes — auth + tenancy + RBAC + CSRF.
    let tenant_routes = OpenApiRouter::new()
        .merge(org::router())
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

    // Credential OAuth callback routes (Plane B — ADR-0031).
    let credential_routes = credential::router().layer(middleware::from_fn_with_state(
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
        .merge(health::router())
        .merge(metrics::router())
        .nest("/api/v1", api_v1)
}
