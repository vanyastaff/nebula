//! OpenAPI 3.1 drift-detection + spec-validation tests (M3.2 Task 7).
//!
//! Boots [`nebula_api::build_app`], fetches `/api/v1/openapi.json`, and
//! verifies the spec is structurally valid and stable:
//!
//! - **Version pin** — `openapi == "3.1.0"` (1).
//! - **operationId uniqueness** — utoipa-axum derives the id from the
//!   handler function name; collisions crash client generators.
//! - **$ref resolution** — every `$ref` under `paths.../responses/.../schema`
//!   resolves to a declared component.
//! - **Required security schemes** — `bearer`, `api_key`, `session_cookie`, and `csrf` are
//!   published per 2.
//! - **Drift parity** — the spec mentions every key path mounted under
//!   `routes::create_routes`; if a handler is added but not annotated
//!   (or vice versa) this test catches it.
//! - **Strict 3.1 round-trip** — the served JSON is parsed back through
//!   the typed `oas3::OpenApi` parser. A failure means the spec cannot
//!   be consumed by 3.1-compliant tooling.

mod common;

use std::collections::HashSet;

use axum::{body::Body, http::Request};
use nebula_api::{
    ApiConfig,
    access::{REQUIRED_PERMISSION_EXTENSION, UNSUPPORTED_PERMISSION_SCOPE},
    build_app,
};
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
async fn credential_oauth_ceremony_is_absent_while_identity_oauth_stays_public() {
    let spec = fetch_spec_json().await;
    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .expect("spec.paths must be present");

    for removed in [
        "/api/v1/credentials/{id}/oauth2/auth",
        "/api/v1/credentials/{id}/oauth2/callback",
        "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/auth",
        "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/callback",
    ] {
        assert!(
            !paths.contains_key(removed),
            "Plane-B OAuth ceremony must not be advertised: {removed}"
        );
    }

    let leaked: Vec<&String> = paths
        .keys()
        .filter(|path| path.contains("/credentials/") && path.contains("/oauth2/"))
        .collect();
    assert!(
        leaked.is_empty(),
        "no credential path may expose an OAuth ceremony; found {leaked:?}"
    );

    for retained in [
        "/api/v1/auth/oauth/{provider}",
        "/api/v1/auth/oauth/{provider}/callback",
    ] {
        let operation = paths
            .get(retained)
            .and_then(|item| item.get("get"))
            .unwrap_or_else(|| panic!("Plane-A OAuth route must remain published: {retained}"));
        let security = operation
            .get("security")
            .and_then(Value::as_array)
            .unwrap_or_else(|| {
                panic!("Plane-A OAuth route must declare empty security: {retained}")
            });
        assert!(
            security.is_empty()
                || security.iter().all(|requirement| requirement
                    .as_object()
                    .is_some_and(serde_json::Map::is_empty)),
            "Plane-A OAuth route must remain unauthenticated: {retained}; security={security:?}"
        );

        let responses = operation
            .get("responses")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("OAuth operation needs typed responses: {retained}"));
        let expected: &[&str] = if retained.ends_with("/callback") {
            &[
                "200", "202", "400", "401", "403", "409", "500", "502", "503",
            ]
        } else {
            &["200", "400", "429", "500", "502", "503"]
        };
        for status in expected {
            assert!(
                responses.contains_key(*status),
                "OAuth operation {retained} is missing honest {status} response"
            );
        }
    }

    for universal_acquisition in [
        "/api/v1/orgs/{org}/workspaces/{ws}/credentials/resolve",
        "/api/v1/orgs/{org}/workspaces/{ws}/credentials/resolve/continue",
    ] {
        let operation = paths
            .get(universal_acquisition)
            .and_then(|item| item.get("post"));
        assert!(
            operation.is_some(),
            "universal Plane-B acquisition route must remain published: {universal_acquisition}"
        );
    }
}

#[tokio::test]
async fn identity_oauth_provider_and_mfa_continuation_are_typed_closed_contracts() {
    let spec = fetch_spec_json().await;
    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .expect("spec.paths must be present");
    let schemas = spec
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .expect("component schemas must be present");

    for path in [
        concat!("/api/v1/auth/oauth/", "{", "provider", "}"),
        concat!("/api/v1/auth/oauth/", "{", "provider", "}", "/callback"),
    ] {
        let operation = paths
            .get(path)
            .and_then(|item| item.get("get"))
            .unwrap_or_else(|| panic!("OAuth GET operation must exist: {path}"));
        let provider = operation
            .get("parameters")
            .and_then(Value::as_array)
            .and_then(|parameters| {
                parameters.iter().find(|parameter| {
                    parameter.get("name").and_then(Value::as_str) == Some("provider")
                })
            })
            .unwrap_or_else(|| panic!("OAuth operation must type its provider parameter: {path}"));
        let schema = provider
            .get("schema")
            .unwrap_or_else(|| panic!("provider parameter must carry a schema: {provider:?}"));
        let schema = schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|reference| reference.rsplit('/').next())
            .and_then(|name| schemas.get(name))
            .unwrap_or(schema);
        let values: HashSet<&str> = schema
            .get("enum")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("provider schema must be a closed enum: {schema:?}"))
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(values, HashSet::from(["google", "github"]));
    }

    let mfa_responses = paths
        .get("/api/v1/auth/login/mfa")
        .and_then(|item| item.get("post"))
        .and_then(|operation| operation.get("responses"))
        .and_then(Value::as_object)
        .expect("MFA login completion must publish responses");
    for status in ["200", "401", "500", "503"] {
        assert!(
            mfa_responses.contains_key(status),
            "MFA login completion must document {status}: {mfa_responses:?}"
        );
    }
}

#[tokio::test]
async fn credential_test_response_is_a_frozen_tagged_v1_union() {
    let spec = fetch_spec_json().await;
    let schemas = spec
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .expect("spec.components.schemas must be present");

    let response = schemas
        .get("TestCredentialResponse")
        .expect("test response schema must be registered");
    let branches = response
        .get("oneOf")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("test response must be a oneOf union: {response}"));
    assert_eq!(
        branches.len(),
        2,
        "v1 test response has exactly success and failed branches: {response}"
    );

    let mut statuses = HashSet::new();
    for branch in branches {
        let properties = branch
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("tagged response branch needs properties: {branch}"));
        let status = properties
            .get("status")
            .and_then(|schema| schema.get("enum"))
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("branch status must be a single literal: {branch}"));
        assert!(
            statuses.insert(status.to_owned()),
            "duplicate tagged response status `{status}`"
        );

        let required: HashSet<&str> = branch
            .get("required")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("tagged response branch needs required fields: {branch}"))
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(required.contains("status"));
        assert!(required.contains("message"));
        assert!(required.contains("tested_at"));
        match status {
            "success" => {
                assert!(!required.contains("code"));
                assert!(!properties.contains_key("code"));
            },
            "failed" => {
                assert!(required.contains("code"));
                assert!(properties.contains_key("code"));
            },
            other => panic!("v1 test response exposed unexpected status `{other}`"),
        }
    }
    assert_eq!(
        statuses,
        HashSet::from(["success".to_owned(), "failed".to_owned()])
    );

    let code_values: Vec<&str> = schemas
        .get("CredentialTestFailureCodeV1")
        .and_then(|schema| schema.get("enum"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("v1 failure-code enum must be registered: {schemas:?}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("failure code must be a string: {value}"))
        })
        .collect();
    assert_eq!(
        code_values,
        [
            "authentication_rejected",
            "permission_denied",
            "account_restricted",
            "invalid_configuration",
            "other",
        ],
        "the exhaustive HTTP v1 failure-code vocabulary is frozen"
    );
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
        "stub-endpoint policy pins the generator to OpenAPI 3.1.0; got {version}"
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
async fn tenant_operations_declare_required_permission() {
    let spec = fetch_spec_json().await;
    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .expect("spec.paths must be present");

    let mut checked = 0usize;
    for (path, item) in paths {
        if !path.starts_with("/api/v1/orgs/{org}") {
            continue;
        }

        let item = item.as_object().expect("each path entry must be an object");
        for method in HTTP_METHODS {
            let Some(operation) = item.get(*method) else {
                continue;
            };
            checked += 1;

            let permission = operation
                .get(REQUIRED_PERMISSION_EXTENSION)
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    panic!(
                        "operation {method} {path} must declare string `{REQUIRED_PERMISSION_EXTENSION}`"
                    )
                });
            assert!(
                !permission.is_empty(),
                "operation {method} {path} must not declare an empty `{REQUIRED_PERMISSION_EXTENSION}`"
            );
            assert_ne!(
                permission, UNSUPPORTED_PERMISSION_SCOPE,
                "operation {method} {path} must not publish unsupported permission placeholder"
            );
        }
    }

    assert!(
        checked > 0,
        "spec must contain at least one tenant operation under /api/v1/orgs/{{org}}"
    );
}

#[tokio::test]
async fn selected_operations_publish_expected_permissions() {
    let spec = fetch_spec_json().await;

    for (method, path, expected) in [
        // Org
        ("get", "/api/v1/orgs/{org}", "orgs:read"),
        ("patch", "/api/v1/orgs/{org}", "orgs:update"),
        ("delete", "/api/v1/orgs/{org}", "orgs:delete"),
        ("get", "/api/v1/orgs/{org}/members", "members:read"),
        ("post", "/api/v1/orgs/{org}/members", "members:invite"),
        (
            "delete",
            "/api/v1/orgs/{org}/members/{principal}",
            "members:remove",
        ),
        // Workspace/workflow
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows",
            "workflows:read",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows",
            "workflows:write",
        ),
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}",
            "workflows:read",
        ),
        (
            "put",
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}",
            "workflows:write",
        ),
        (
            "delete",
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}",
            "workflows:delete",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/execute",
            "workflows:execute",
        ),
        // Executions
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/executions",
            "executions:read",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions",
            "workflows:execute",
        ),
        (
            "delete",
            "/api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}",
            "executions:cancel",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/terminate",
            "executions:terminate",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/restart",
            "executions:restart",
        ),
        // Resources
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/resources",
            "resources:read",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/resources",
            "resources:write",
        ),
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/resources/{res}",
            "resources:read",
        ),
        (
            "put",
            "/api/v1/orgs/{org}/workspaces/{ws}/resources/{res}",
            "resources:write",
        ),
        (
            "delete",
            "/api/v1/orgs/{org}/workspaces/{ws}/resources/{res}",
            "resources:delete",
        ),
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/resources/{res}/status",
            "resources:read",
        ),
        // Credentials
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials",
            "credentials:read",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials",
            "credentials:write",
        ),
        (
            "get",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}",
            "credentials:read",
        ),
        (
            "put",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}",
            "credentials:write",
        ),
        (
            "delete",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}",
            "credentials:delete",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials/resolve",
            "credentials:write",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/test",
            "credentials:write",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh",
            "credentials:write",
        ),
        (
            "post",
            "/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke",
            "credentials:delete",
        ),
    ] {
        assert_eq!(
            required_permission_for(&spec, path, method),
            expected,
            "{method} {path} must publish `{expected}` as `{REQUIRED_PERMISSION_EXTENSION}`"
        );
    }
}

fn required_permission_for<'a>(spec: &'a Value, path: &str, method: &str) -> &'a str {
    spec.get("paths")
        .and_then(|paths| paths.get(path))
        .and_then(|item| item.get(method))
        .and_then(|operation| operation.get(REQUIRED_PERMISSION_EXTENSION))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!(
                "operation {method} {path} must exist and declare string `{REQUIRED_PERMISSION_EXTENSION}`"
            )
        })
}

#[tokio::test]
async fn security_schemes_match_adr_0047() {
    let spec = fetch_spec_json().await;
    let schemes = spec
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .and_then(Value::as_object)
        .expect("components.securitySchemes must be present per OpenAPI contract");

    for required in ["bearer", "api_key", "session_cookie", "csrf"] {
        assert!(
            schemes.contains_key(required),
            "OpenAPI contract mandates security scheme `{required}` in the spec; \
             got {:?}",
            schemes.keys().collect::<Vec<_>>()
        );
    }

    assert_eq!(
        schemes["session_cookie"].get("in").and_then(Value::as_str),
        Some("cookie")
    );
    assert_eq!(
        schemes["session_cookie"]
            .get("name")
            .and_then(Value::as_str),
        Some("__Host-nebula-session")
    );
    assert_eq!(
        schemes["csrf"].get("in").and_then(Value::as_str),
        Some("header")
    );
    assert_eq!(
        schemes["csrf"].get("name").and_then(Value::as_str),
        Some("x-csrf-token")
    );
    assert!(
        !schemes.contains_key("session"),
        "there must be one canonical cookie-session scheme"
    );
}

#[tokio::test]
async fn protected_operations_publish_explicit_and_cookie_auth_alternatives() {
    let spec = fetch_spec_json().await;

    let requirement_keys = |method: &str| {
        let mut requirements = spec["paths"]["/api/v1/me"][method]["security"]
            .as_array()
            .unwrap_or_else(|| panic!("{method} /api/v1/me must publish security requirements"))
            .iter()
            .map(|requirement| {
                let mut keys = requirement
                    .as_object()
                    .expect("security requirement must be an object")
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                keys.sort_unstable();
                keys
            })
            .collect::<Vec<_>>();
        requirements.sort_unstable();
        requirements
    };

    assert_eq!(
        requirement_keys("get"),
        vec![
            vec!["api_key".to_owned()],
            vec!["bearer".to_owned()],
            vec!["session_cookie".to_owned()],
        ],
        "safe session-auth operations do not require CSRF"
    );
    assert_eq!(
        requirement_keys("patch"),
        vec![
            vec!["api_key".to_owned()],
            vec!["bearer".to_owned()],
            vec!["csrf".to_owned(), "session_cookie".to_owned()],
        ],
        "mutating session-auth operations require session_cookie AND csrf"
    );
}

#[tokio::test]
async fn mfa_enrollment_requires_session_cookie_and_csrf_together() {
    let spec = fetch_spec_json().await;
    for path in ["/api/v1/auth/mfa/enroll", "/api/v1/auth/mfa/verify"] {
        let requirements = spec["paths"][path]["post"]["security"]
            .as_array()
            .unwrap_or_else(|| panic!("{path} must publish security requirements"));
        assert_eq!(
            requirements.len(),
            1,
            "{path} must not publish bearer/API-key alternatives"
        );
        let requirement = requirements[0]
            .as_object()
            .unwrap_or_else(|| panic!("{path} security requirement must be an object"));
        assert_eq!(
            requirement
                .keys()
                .map(String::as_str)
                .collect::<HashSet<_>>(),
            HashSet::from(["session_cookie", "csrf"]),
            "{path} requires both ambient session authority and CSRF proof"
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
        "/api/v1/orgs/{org}/workspaces/{ws}/resources",
        // The single-resource config path: GET (read) / PUT
        // (CAS update) / DELETE (soft-delete). Pinned so the
        // config-CRUD surface is drift-detected alongside the
        // collection and status paths.
        "/api/v1/orgs/{org}/workspaces/{ws}/resources/{res}",
        // The resource runtime-status read is the ONLY `{res}`
        // sub-route, and it is GET-only: resource lifecycle
        // (acquire/release/drain/reload) is engine-owned and
        // deliberately never exposed over HTTP (INTEGRATION_MODEL
        // integration seam.1). Pinning the status path here makes that
        // "observe-only, no lifecycle verbs" boundary drift-detected —
        // if a mutating `{res}` sub-route is ever added, the contract
        // change is visible against this list.
        "/api/v1/orgs/{org}/workspaces/{ws}/resources/{res}/status",
        // Webhooks: `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}` is
        // mounted directly by `WebhookTransport::router()` (a plain
        // axum Router, not an OpenApiRouter) so it does not appear in
        // the served OpenAPI spec — operator-configured webhook URLs
        // are documented in the README and webhook activation, not the spec.
        // The slug surface is consumed by external providers, not by
        // SDK / Swagger users; promoting it back into the spec would
        // require typed schemas for every provider's request envelope.
        // webhook activation § "Out of scope (1.0 follow-ups)" tracks the
        // re-promotion option.
    ];

    for path in expected {
        assert!(
            paths.contains_key(path),
            "Drift smoke: path `{path}` is missing from the served spec. \
             Either the handler / annotation was removed or the route \
             registration changed without updating this list."
        );
    }

    // The resource runtime-status sub-route is **observe-only**: it must
    // expose ONLY a `get` operation. Resource lifecycle
    // (acquire/release/drain/reload) is engine-owned and deliberately
    // never exposed over HTTP (INTEGRATION_MODEL integration seam.1), so an accidental
    // mutating verb on this path is a contract regression — fail drift
    // here rather than silently shipping a lifecycle write endpoint.
    let status_item = paths
        .get("/api/v1/orgs/{org}/workspaces/{ws}/resources/{res}/status")
        .and_then(Value::as_object)
        .expect("the resource status path item must be an object");
    let methods: Vec<&String> = status_item
        .keys()
        // OpenAPI path-item keys include non-operation fields
        // (`parameters`, `summary`, …); only the HTTP verbs are operations.
        .filter(|k| {
            matches!(
                k.as_str(),
                "get" | "put" | "post" | "delete" | "patch" | "head" | "options" | "trace"
            )
        })
        .collect();
    assert_eq!(
        methods,
        vec!["get"],
        "the resource `/status` sub-route must be GET-only (observe-only; \
         no lifecycle mutation over HTTP — INTEGRATION_MODEL §13.1); a \
         non-`get` operation here is a contract regression. Found: {methods:?}"
    );
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
