//! OpenAPI 3.1 spec generation for `nebula-api`.
//!
//! See [ADR-0047](../../../../docs/adr/0047-openapi-31-generator.md) for
//! the library choice rationale, drift-detection guarantees, stub-endpoint
//! policy, and cross-layer schema strategy.
//!
//! ## Architecture
//!
//! - [`OpenApiDoc`] is the `#[derive(OpenApi)]` value passed to
//!   `utoipa_axum::router::OpenApiRouter::with_openapi`. The derive captures
//!   document metadata (title, description, version, contact) and registers
//!   the `SecuritySchemes` modifier that publishes the three Plane-A
//!   authentication schemes (Bearer JWT, `X-API-Key`, CSRF cookie).
//! - Handler annotations (`#[utoipa::path]`) and DTO derives (`#[derive(ToSchema)]`)
//!   are added in subsequent tasks. The `OpenApiRouter` mounting
//!   path collects them automatically — handlers without `#[utoipa::path]`
//!   cannot pass through `routes!()`, so drift is a compile error.
//! - The materialized `OpenApi` value will be cached in `AppState::openapi_doc`
//!   so the runtime `GET /api/v1/openapi.json` handler returns it without
//!   re-deriving on every request.
//!
//! ## Security schemes
//!
//! - **`bearer`** (HTTP Bearer JWT): the standard JWT cookie / `Authorization: Bearer …`
//!   path. Issued by the Plane-A auth backend; validated by
//!   [`crate::middleware::auth`].
//! - **`api_key`** (`X-API-Key` header): static API keys with the
//!   `nbl_sk_` prefix. Validated in constant time alongside JWT.
//! - **`csrf`** (`X-CSRF-Token` header / `__Host-csrf` cookie): double-submit
//!   token enforced by [`crate::middleware::csrf`] for mutating requests on
//!   cookie-authenticated sessions.
//!
//! Public endpoints (`/health`, `/ready`, `/version`, `/metrics`,
//! `/api/v1/openapi.json`, `/api/v1/docs`, all `/api/v1/auth/*`,
//! `/api/v1/hooks/*`) opt out via `security(())` on their `#[utoipa::path]`.

use utoipa::{
    Modify, OpenApi,
    openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme},
};

/// Root OpenAPI 3.1 document for `nebula-api`.
///
/// Materialized via [`OpenApiDoc::openapi()`] (provided by the
/// `#[derive(OpenApi)]` macro). At composition time, `app::build_app`
/// passes this value to `utoipa_axum::router::OpenApiRouter::with_openapi`
/// and lets the router collect handler paths through `routes!()`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Nebula API",
        description = "HTTP API for the Nebula workflow automation engine. \
                       Stub endpoints (those tagged `(planned)` and marked \
                       `deprecated`) currently return 501; payload schemas \
                       describe the planned shape per ROADMAP.",
        version = env!("CARGO_PKG_VERSION"),
        contact(
            name = "Nebula maintainers",
            url = "https://github.com/vanyastaff/nebula",
        ),
        license(
            name = "MIT OR Apache-2.0",
            url = "https://github.com/vanyastaff/nebula#license",
        ),
    ),
    servers(
        (url = "/", description = "Same-origin"),
    ),
    modifiers(&SecuritySchemes),
)]
pub struct OpenApiDoc;

/// `Modify` impl that publishes the three Plane-A security schemes.
///
/// Declaring schemes via a `Modify` impl (rather than the derive's
/// `components(security_schemes(...))` form) keeps the scheme construction
/// in one place and avoids the macro-attribute escape-hatch dance for the
/// CSRF cookie + header pair.
struct SecuritySchemes;

impl Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new);

        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .description(Some(
                        "Plane-A bearer JWT issued by the auth backend. \
                         Pass via `Authorization: Bearer <jwt>` or a session cookie.",
                    ))
                    .build(),
            ),
        );

        components.add_security_scheme(
            "api_key",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "X-API-Key",
                "Static API key with the `nbl_sk_` prefix. Compared in \
                 constant time. Tenant-scoped.",
            ))),
        );

        components.add_security_scheme(
            "csrf",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "X-CSRF-Token",
                "Double-submit CSRF token. Required on mutating requests \
                 authenticated via cookie. The token MUST match the value \
                 in the `__Host-csrf` cookie.",
            ))),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Materializing the OpenAPI value must succeed at compile/test time.
    /// This locks the `#[derive(OpenApi)]` shape and the `Modify` impl
    /// against accidental breakage; the route-table parity / drift tests
    /// land in T7 (`crates/api/tests/openapi_spec.rs`).
    #[test]
    fn openapi_doc_materializes() {
        let openapi = OpenApiDoc::openapi();

        assert_eq!(openapi.info.title, "Nebula API");
        assert!(
            !openapi.info.version.is_empty(),
            "version must be wired from CARGO_PKG_VERSION"
        );

        let components = openapi
            .components
            .as_ref()
            .expect("security schemes registered via modifier");
        assert!(components.security_schemes.contains_key("bearer"));
        assert!(components.security_schemes.contains_key("api_key"));
        assert!(components.security_schemes.contains_key("csrf"));
    }
}
