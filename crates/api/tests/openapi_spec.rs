//! OpenAPI 3.1 drift-detection + spec-validation tests (M3.2 Task 7).
//!
//! Boots [`nebula_api::build_app`], fetches `/api/v1/openapi.json`, and
//! verifies the spec is structurally valid and stable:
//!
//! - **Version pin** — `openapi == "3.1.0"` (ADR-0047 §1).
//! - **operationId uniqueness** — utoipa-axum derives the id from the
//!   handler function name; collisions crash client generators.
//! - **$ref resolution** — every `$ref` under `paths.../responses/.../schema`
//!   resolves to a declared component.
//! - **Required security schemes** — `bearer`, `api_key`, and `csrf` are
//!   published per ADR-0047 §2.
//! - **Drift parity** — the spec mentions every key path mounted under
//!   `routes::create_routes`; if a handler is added but not annotated
//!   (or vice versa) this test catches it.
//! - **Strict 3.1 round-trip** — the served JSON is parsed back through
//!   the typed `oas3::OpenApi` parser. A failure means the spec cannot
//!   be consumed by 3.1-compliant tooling.

mod common;

use std::collections::HashSet;

use axum::{body::Body, http::Request};
use nebula_api::{ApiConfig, build_app};
use serde_json::Value;
use tower::ServiceExt;

use crate::common::create_state_with_queue;

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "options", "head", "trace",
];

async fn fetch_spec_json() -> Value {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = build_app(state, &config);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("openapi.json must respond");
    assert_eq!(response.status(), 200, "openapi.json must return 200");

    let body = axum::body::to_bytes(response.into_body(), 8 * 1_000_000)
        .await
        .expect("body must be readable");
    serde_json::from_slice(&body).expect("openapi.json body must be valid JSON")
}

#[tokio::test]
async fn served_spec_pins_openapi_3_1_0() {
    let spec = fetch_spec_json().await;
    let version = spec
        .get("openapi")
        .and_then(Value::as_str)
        .expect("spec.openapi must be a string");
    assert_eq!(
        version, "3.1.0",
        "ADR-0047 pins the generator to OpenAPI 3.1.0; got {version}"
    );
}

#[tokio::test]
async fn operation_ids_are_unique() {
    let spec = fetch_spec_json().await;
    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .expect("spec.paths must be present");

    let mut seen: HashSet<String> = HashSet::new();
    for (path, ops) in paths {
        let ops = ops.as_object().expect("each path entry must be an object");
        for (method, op) in ops {
            if !HTTP_METHODS.contains(&method.as_str()) {
                continue;
            }
            let op_id = op
                .get("operationId")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("operation {method} {path} must declare operationId"));
            assert!(
                seen.insert(op_id.to_owned()),
                "operationId `{op_id}` is duplicated (collision at {method} {path}); \
                 every Rust handler function name must be unique to satisfy \
                 the operationId-as-fn-name contract from utoipa-axum"
            );
        }
    }
    assert!(
        !seen.is_empty(),
        "spec must register at least one operation"
    );
}

#[tokio::test]
async fn all_refs_resolve_to_declared_components() {
    let spec = fetch_spec_json().await;
    let schemas: HashSet<String> = spec
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(Value::as_object)
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    let mut unresolved: Vec<String> = Vec::new();
    walk_refs(&spec, &schemas, &mut unresolved);

    assert!(
        unresolved.is_empty(),
        "Found unresolved $ref(s) in served spec: {unresolved:#?}"
    );
}

fn walk_refs(value: &Value, schemas: &HashSet<String>, unresolved: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "$ref" {
                    if let Some(target) = child.as_str()
                        && let Some(name) = target.strip_prefix("#/components/schemas/")
                        && !schemas.contains(name)
                    {
                        unresolved.push(target.to_owned());
                    }
                } else {
                    walk_refs(child, schemas, unresolved);
                }
            }
        },
        Value::Array(items) => {
            for item in items {
                walk_refs(item, schemas, unresolved);
            }
        },
        _ => {},
    }
}

#[tokio::test]
async fn security_schemes_match_adr_0047() {
    let spec = fetch_spec_json().await;
    let schemes = spec
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .and_then(Value::as_object)
        .expect("components.securitySchemes must be present per ADR-0047 §2");

    for required in ["bearer", "api_key", "csrf"] {
        assert!(
            schemes.contains_key(required),
            "ADR-0047 mandates security scheme `{required}` in the spec; \
             got {:?}",
            schemes.keys().collect::<Vec<_>>()
        );
    }
}

/// Smoke check that key public paths are present in the spec — drift
/// detection for the most load-bearing endpoints. If a handler is
/// removed/renamed without updating the route module, this fails fast.
///
/// `/api/v1/openapi.json` and `/api/v1/docs/` are intentionally **not**
/// in the list: they are served by `utoipa_swagger_ui::SwaggerUi` as a
/// Tower service (no `#[utoipa::path]` annotation), so they will not
/// appear in `paths`. Their reachability is exercised by
/// [`swagger_ui_endpoints_are_reachable`] below.
#[tokio::test]
async fn drift_smoke_known_paths_are_present() {
    let spec = fetch_spec_json().await;
    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .expect("spec.paths must be present");

    let expected = [
        // System
        "/health",
        "/ready",
        "/version",
        // Auth
        "/api/v1/auth/signup",
        "/api/v1/auth/login",
        "/api/v1/auth/logout",
        // Catalog
        "/api/v1/actions",
        "/api/v1/plugins",
        // Tenant
        "/api/v1/orgs/{org}/workspaces/{ws}/workflows",
        "/api/v1/orgs/{org}/workspaces/{ws}/executions",
        "/api/v1/orgs/{org}/workspaces/{ws}/credentials",
        // Webhooks
        "/api/v1/hooks/{org}/{ws}/{trigger_slug}",
    ];

    for path in expected {
        assert!(
            paths.contains_key(path),
            "Drift smoke: path `{path}` is missing from the served spec. \
             Either the handler / annotation was removed or the route \
             registration changed without updating this list."
        );
    }
}

/// Probe the two SwaggerUi-served endpoints — `/api/v1/openapi.json`
/// (spec JSON) and `/api/v1/docs/` (Swagger UI HTML) — for HTTP
/// reachability. Spec-self-doesn't-document-itself is acceptable per
/// [`drift_smoke_known_paths_are_present`]; this test prevents the
/// SwaggerUi mount from silently regressing.
#[tokio::test]
async fn swagger_ui_endpoints_are_reachable() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = build_app(state, &config);

    // 1. Spec JSON
    let spec_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("openapi.json must respond");
    assert_eq!(
        spec_response.status(),
        200,
        "/api/v1/openapi.json must return 200 from the SwaggerUi tower service"
    );

    // 2. Swagger UI HTML page (with trailing slash — SwaggerUi serves
    //    the index there; the bare `/api/v1/docs` may issue a 301/308
    //    redirect to the trailing-slash form).
    let docs_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/docs/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("docs page must respond");
    assert!(
        docs_response.status().is_success(),
        "/api/v1/docs/ must return a 2xx status from the SwaggerUi tower service; got {}",
        docs_response.status()
    );
}

#[tokio::test]
async fn served_spec_round_trips_through_oas3_parser() {
    let spec = fetch_spec_json().await;
    let json_bytes = serde_json::to_vec(&spec).expect("spec must re-serialize");

    // Parse the served JSON through the typed `oas3::OpenApiV3Spec`
    // representation (oas3 0.16 supports both 3.0 and 3.1 documents).
    // A failure here means the spec cannot be round-tripped by a strict
    // 3.1 parser, which downstream client generators (openapi-generator,
    // oapi-codegen, etc.) would also reject.
    let parsed: oas3::OpenApiV3Spec =
        serde_json::from_slice(&json_bytes).expect("served spec must parse as oas3::OpenApiV3Spec");

    let path_count = parsed
        .paths
        .as_ref()
        .map_or(0, std::collections::BTreeMap::len);
    assert!(path_count > 0, "oas3 parser must observe at least one path");
    assert!(
        !parsed.info.title.is_empty(),
        "oas3 parser must observe a non-empty title"
    );
}
