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
//! the `/api/v1` prefix is applied via `OpenApiRouter::nest` in
//! [`build_openapi_router`] so the published spec and the served route
//! table share one source of truth (ADR-0047 drift-detection guarantee).

pub mod auth;
pub mod catalog;
pub mod health;
pub mod me;
pub mod metrics;
pub mod openapi;
pub mod org;
pub mod webhook;
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
/// `OpenApi` is the merged spec for `GET /api/v1/openapi.json` (cached on
/// `AppState` via [`AppState::install_openapi_doc`]).
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

    // OpenAPI docs — no auth.
    let docs_routes = openapi::router();

    // Webhook routes — special: no standard auth, separate per-trigger
    // authentication enforced inside the dispatcher.
    let webhook_routes = webhook::router();

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

    // Compose `/api/v1` group.
    let api_v1 = OpenApiRouter::new()
        .merge(auth_routes)
        .merge(docs_routes)
        .merge(webhook_routes)
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
