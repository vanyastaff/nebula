//! OpenAPI 3.1 spec generation for `nebula-api`.
//!
//! See [stub-endpoint policy](../../../../) for
//! the library choice rationale, drift-detection guarantees, stub-endpoint
//! policy, and cross-layer schema strategy.
//!
//! ## Architecture
//!
//! - [`OpenApiDoc`] is the `#[derive(OpenApi)]` value passed to
//!   `utoipa_axum::router::OpenApiRouter::with_openapi`. The derive captures
//!   document metadata (title, description, version, contact) and registers
//!   the `SecuritySchemes` modifier that publishes the four Plane-A
//!   authentication schemes (Bearer JWT/PAT, `X-API-Key`, session cookie,
//!   and CSRF header).
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
//! - **`bearer`** (HTTP Bearer JWT or PAT): explicit
//!   `Authorization: Bearer …` authority, validated by
//!   [`crate::middleware::auth`].
//! - **`api_key`** (`X-API-Key` header): static API keys with the
//!   `nbl_sk_` prefix. Validated in constant time alongside JWT.
//! - **`session_cookie`** (`__Host-nebula-session` cookie): ambient browser
//!   authority. Mutating operations pair it with `csrf` in one OpenAPI
//!   security requirement.
//! - **`csrf`** (`X-CSRF-Token` header / `__Host-nebula-csrf` cookie): double-submit
//!   token enforced by [`crate::middleware::csrf`] for mutating requests on
//!   cookie-authenticated sessions.
//!
//! Public endpoints (`/health`, `/ready`, `/version`, `/metrics`,
//! `/api/v1/openapi.json`, `/api/v1/docs`, cookie-less auth initiation and
//! completion routes, `/api/v1/hooks/*`) opt out via `security(())` on their
//! `#[utoipa::path]`. MFA enrollment and confirmation are protected exceptions.

use utoipa::{
    Modify, OpenApi,
    openapi::{
        path::Operation,
        security::{
            ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme,
        },
    },
};

use crate::domain::auth::backend::{CSRF_HEADER, SESSION_COOKIE};

/// Unsupported, explicitly activated runtime conformance instrumentation.
#[cfg(feature = "test-util")]
#[doc(hidden)]
pub mod conformance;

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

/// `Modify` impl that publishes the four Plane-A security schemes.
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
                    .bearer_format("JWT or pat_…")
                    .description(Some(
                        "Explicit Plane-A bearer credential: either an HS256 JWT or a \
                         personal access token with the `pat_` prefix. Explicit headers \
                         take precedence over ambient session cookies and fail closed.",
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
            "session_cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                SESSION_COOKIE,
                "Host-bound browser session. Always Secure, HttpOnly, SameSite=Lax, Path=/, \
                 with no Domain attribute. Mutating requests also require the `csrf` scheme; \
                 sensitive identity mutations may additionally require a fresh session. \
                 This lane is schemeful same-site only; cross-site clients must use Bearer \
                 authentication even when their origin is admitted by CORS.",
            ))),
        );

        components.add_security_scheme(
            "csrf",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                CSRF_HEADER,
                "Double-submit CSRF token. Required on mutating requests \
                 authenticated via the `session_cookie`. The token MUST match the value \
                 in the `__Host-nebula-csrf` cookie.",
            ))),
        );
    }
}

/// Add the cookie-session alternative to every operation already marked as
/// protected by an explicit header credential.
///
/// Handler annotations remain the source of truth for whether an operation is
/// protected. This post-merge pass derives the second supported protocol lane:
/// `session_cookie` alone for safe methods, and the AND-pair
/// `session_cookie` + `csrf` for
/// mutating methods. Keeping this derivation centralized prevents dozens of
/// annotations from drifting away from the actual middleware contract.
pub(crate) fn add_session_security(openapi: &mut utoipa::openapi::OpenApi) {
    for item in openapi.paths.paths.values_mut() {
        add_session_requirement(&mut item.get, false);
        add_session_requirement(&mut item.head, false);
        add_session_requirement(&mut item.options, false);
        add_session_requirement(&mut item.trace, false);
        add_session_requirement(&mut item.post, true);
        add_session_requirement(&mut item.put, true);
        add_session_requirement(&mut item.patch, true);
        add_session_requirement(&mut item.delete, true);
    }
}

/// Match the served error contract to the shared RFC 9457 runtime.
///
/// Handlers declare their own outcomes; this pass adds the failures that
/// router-wide layers produce before or around any handler, so the document
/// names every status the live router can return:
///
/// - `429` from the global per-IP limiter on every non-probe route;
/// - `404`/`503` from tenant resolution on every `/orgs/{org}` route;
/// - `400`/`413`/`415`/`422` from the body limit and [`ApiJson`] decoding on
///   every operation that takes a JSON request body.
///
/// Existing, endpoint-specific descriptions are kept.
///
/// [`ApiJson`]: crate::extractors::ApiJson
pub(crate) fn add_problem_response_contract(openapi: &mut utoipa::openapi::OpenApi) {
    const TENANT_PREFIX: &str = "/api/v1/orgs/{org}";
    for (path, item) in &mut openapi.paths.paths {
        for operation in [
            &mut item.get,
            &mut item.head,
            &mut item.options,
            &mut item.trace,
            &mut item.post,
            &mut item.put,
            &mut item.patch,
            &mut item.delete,
        ]
        .into_iter()
        .flatten()
        {
            normalize_problem_responses(operation);
            let mut layer_failures = Vec::new();
            if !crate::middleware::rate_limit::EXCLUDED_PATHS.contains(&path.as_str()) {
                layer_failures.push((
                    "429",
                    "Global per-IP request quota exceeded; Retry-After carries retry advice.",
                ));
            }
            if path.starts_with(TENANT_PREFIX) {
                layer_failures.extend([
                    (
                        "404",
                        "Organisation or workspace is unknown or not visible to the caller.",
                    ),
                    (
                        "503",
                        "Tenant authority is not configured on this instance.",
                    ),
                ]);
            }
            let takes_json_body = operation
                .request_body
                .as_ref()
                .is_some_and(|body| body.content.contains_key("application/json"));
            if takes_json_body {
                layer_failures.extend([
                    ("400", "Request body is not a valid JSON document."),
                    ("413", "Request body exceeds the configured body limit."),
                    ("415", "Request body is not sent as application/json."),
                    (
                        "422",
                        "Request body does not match the operation's request schema.",
                    ),
                ]);
            }
            for (status, description) in layer_failures {
                operation
                    .responses
                    .responses
                    .entry(status.to_owned())
                    .or_insert_with(|| problem_response(description));
            }
        }
    }
}

fn problem_response(
    description: &str,
) -> utoipa::openapi::RefOr<utoipa::openapi::response::Response> {
    use utoipa::openapi::{ContentBuilder, Ref, ResponseBuilder};
    ResponseBuilder::new()
        .description(description)
        .content(
            "application/problem+json",
            ContentBuilder::new()
                .schema(Some(Ref::from_schema_name("ProblemDetails")))
                .build(),
        )
        .build()
        .into()
}

/// RFC 9457 is the runtime error envelope, including honest 501 stubs.
/// Preserve endpoint-specific error descriptions and the readiness 503 payload.
fn normalize_problem_responses(operation: &mut Operation) {
    use utoipa::openapi::{Ref, RefOr};
    for (status, response) in &mut operation.responses.responses {
        let RefOr::T(response) = response else {
            continue;
        };
        let is_stub = status == "501";
        let is_problem = response.content.values().any(|content| {
            matches!(&content.schema, Some(RefOr::Ref(reference))
                if reference.ref_location == "#/components/schemas/ProblemDetails")
        });
        if !is_stub && !is_problem {
            continue;
        }
        if let Some(mut content) = response.content.shift_remove("application/json") {
            content.schema = Some(Ref::from_schema_name("ProblemDetails").into());
            response
                .content
                .insert("application/problem+json".to_owned(), content);
        }
    }
}

fn add_session_requirement(operation: &mut Option<Operation>, csrf_required: bool) {
    let Some(operation) = operation else {
        return;
    };
    let Some(requirements) = operation.security.as_mut() else {
        return;
    };
    let bearer = SecurityRequirement::new("bearer", std::iter::empty::<&str>());
    let api_key = SecurityRequirement::new("api_key", std::iter::empty::<&str>());
    if !requirements
        .iter()
        .any(|requirement| requirement == &bearer || requirement == &api_key)
    {
        return;
    }

    let mut session = SecurityRequirement::new("session_cookie", std::iter::empty::<&str>());
    if csrf_required {
        session = session.add("csrf", std::iter::empty::<&str>());
    }
    if !requirements.contains(&session) {
        requirements.push(session);
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
        assert!(components.security_schemes.contains_key("session_cookie"));
        assert!(components.security_schemes.contains_key("csrf"));
    }
}
