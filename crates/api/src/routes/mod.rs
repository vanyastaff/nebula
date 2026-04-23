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

pub mod auth;
pub mod catalog;
pub mod health;
pub mod me;
pub mod metrics;
pub mod openapi;
pub mod org;
pub mod webhook;
pub mod workspace;

// Keep existing route modules that are feature-gated
#[cfg(feature = "credential-oauth")]
pub mod credential;

use axum::{Router, middleware};

use crate::{
    config::ApiConfig,
    middleware::{
        auth::auth_middleware, csrf::csrf_middleware, rbac::rbac_middleware,
        tenancy::tenancy_middleware,
    },
    state::AppState,
};

/// Create the complete API router with all routes and middleware layers.
pub fn create_routes(state: AppState, _config: &ApiConfig) -> Router {
    Router::new()
        // Health/internal endpoints — no auth, no tenant scope
        .merge(health::router())
        // Prometheus metrics (no auth required — scraper access)
        .merge(metrics::router())
        // API v1 endpoints
        .nest("/api/v1", api_v1_routes(state.clone()))
        .with_state(state)
}

/// Build the /api/v1 route tree.
fn api_v1_routes(state: AppState) -> Router<AppState> {
    // Auth routes — no auth middleware, no tenant scope
    let auth_routes = auth::router();

    // OpenAPI docs — no auth
    let docs_routes = openapi::router();

    // Webhook routes — special: no standard auth, separate per-trigger auth
    let webhook_routes = webhook::router();

    // User routes — auth required, no tenant scope
    let me_routes = me::router()
        .layer(middleware::from_fn(csrf_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Catalog routes — auth required, no tenant scope
    let catalog_routes = catalog::router().layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
    ));

    // Tenant-scoped routes — auth + tenancy + RBAC + CSRF
    let tenant_routes = Router::new()
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
        .layer(middleware::from_fn(csrf_middleware));

    // Feature-gated credential-oauth routes (Plane B)
    #[allow(unused_mut)]
    let mut router = Router::new()
        .merge(auth_routes)
        .merge(docs_routes)
        .merge(webhook_routes)
        .merge(me_routes)
        .merge(catalog_routes);

    #[cfg(feature = "credential-oauth")]
    {
        let credential_routes = credential::router().layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));
        router = router.merge(credential_routes);
    }

    // Apply auth middleware to tenant routes — state is moved here (last usage).
    let tenant_routes = tenant_routes.layer(middleware::from_fn_with_state(state, auth_middleware));

    router.merge(tenant_routes)
}
