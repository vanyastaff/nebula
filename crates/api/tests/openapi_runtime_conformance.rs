//! NS15 OpenAPI-to-runtime conformance over the shared transport contract.
//!
//! Every case here drives the production router from `build_app`, builds its
//! request from `nebula-api-contract::v1` types, and checks the live response
//! against the *served* `/api/v1/openapi.json`: documented status, documented
//! media type, JSON Schema 2020-12 conformance and decoding into the contract
//! type. A case that produces any drift finding fails on its own.
//!
//! The suite is also the coverage half of the `openapi-runtime-compatibility`
//! producer. With `NEBULA_OPENAPI_OBSERVATIONS` set, the router records every
//! response of every API fixture suite; the ignored
//! `emit_openapi_runtime_compatibility_report` test then aggregates them. The
//! cases below exist so that each documented success status of each served
//! operation is reached by at least one fully consumed, conformant response.
//! Nothing that could carry a payload enters the artifact.
mod common;

use std::{fs, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request},
};
use nebula_api::{
    ApiConfig, AppState, build_app,
    domain::auth::backend::{AuthBackend, InMemoryAuthBackend, mfa},
    openapi::conformance::validation::{
        Observation, Operation, inventory, report, validate_response,
    },
};
use nebula_api_contract::v1;
use serde::Serialize;
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{
    TEST_ORG, TEST_WS, create_state_with_queue, create_test_jwt, make_valid_workflow_definition,
    ws_path,
};

const EXCLUSIONS: &str = include_str!("openapi_runtime_conformance/exclusions.json");
const BODY_CAP: usize = 8 * 1024 * 1024;

fn exclusions() -> Value {
    serde_json::from_str(EXCLUSIONS).unwrap()
}

/// A permissive per-IP quota: these cases exercise handlers, not the limiter.
fn config() -> ApiConfig {
    let mut config = ApiConfig::for_test();
    config.rate_limit_per_second = 10_000;
    config
}

async fn fetch_spec(app: &Router) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), BODY_CAP)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn operation(spec: &Value, id: &str) -> Operation {
    inventory(spec)
        .unwrap()
        .into_keys()
        .find(|operation| operation.operation_id == id)
        .unwrap_or_else(|| panic!("served operation missing: {id}"))
}

fn wire<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap()
}

/// Send `request` as operation `id`, require `expected`, consume the whole
/// body and require zero drift findings. Returns headers and decoded JSON.
async fn exchange(
    app: &Router,
    spec: &Value,
    id: &str,
    request: Request<Body>,
    expected: u16,
) -> (HeaderMap, Value) {
    let operation = operation(spec, id);
    assert_eq!(
        request.method().as_str().to_ascii_lowercase(),
        operation.method,
        "{id}"
    );
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let media = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), BODY_CAP)
        .await
        .unwrap();
    assert_eq!(
        status,
        expected,
        "{id}: {}",
        String::from_utf8_lossy(&bytes)
    );
    let observation = validate_response(spec, &operation, status, media.as_deref(), &bytes);
    assert!(
        observation.findings.is_empty(),
        "{id}: {:?}",
        observation.findings
    );
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (headers, body)
}

/// A request with optional bearer authority and optional JSON body.
fn request(method: &str, uri: &str, bearer: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// Drift findings the report attributes to `operation_id`.
fn findings_for(report: &Value, operation_id: &str) -> Vec<String> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["operation_id"] == operation_id)
        .map(|finding| finding["kind"].as_str().unwrap().to_owned())
        .collect()
}

// ── Producer fail-closed controls ────────────────────────────────────────────

#[test]
fn explicit_observer_write_failures_fail_the_test_process() {
    let executable = std::env::current_exe().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    for (case, setup) in [("initialization", "file"), ("retention", "directory")] {
        let directory = temporary.path().join(case);
        if setup == "file" {
            fs::write(&directory, b"not a directory").unwrap();
        } else {
            fs::create_dir(&directory).unwrap();
        }
        let status = std::process::Command::new(&executable)
            .args(["--exact", "producer_write_failure_child", "--ignored"])
            .env("NEBULA_OPENAPI_OBSERVATIONS", &directory)
            .env("NEBULA_OPENAPI_FAILURE_CONTROL", case)
            .status()
            .unwrap();
        assert_eq!(
            status.code(),
            Some(86),
            "lost evidence must fail closed: {case}"
        );
    }
}

#[tokio::test]
#[ignore = "subprocess-only fail-closed control"]
async fn producer_write_failure_child() {
    let Some(control) = std::env::var_os("NEBULA_OPENAPI_FAILURE_CONTROL") else {
        return;
    };
    let app = build_app(common::build_me_state(), &ApiConfig::for_test());
    if control == "retention" {
        let directory =
            std::path::PathBuf::from(std::env::var_os("NEBULA_OPENAPI_OBSERVATIONS").unwrap());
        let retained = directory.with_extension("retained");
        fs::rename(&directory, &retained).unwrap();
        fs::write(&directory, b"not a directory").unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let _ = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
    }
    panic!("explicit observation write failure did not fail the process");
}

// ── Falsifiability ───────────────────────────────────────────────────────────

/// The gate must not pass vacuously: dropping a response schema from the
/// served document, renaming a contract field, or drifting the runtime body
/// each turn one live `/version` response into a nonzero drift count.
#[tokio::test]
async fn removed_schemas_and_renamed_fields_make_the_drift_count_nonzero() {
    let app = build_app(common::build_me_state(), &config());
    let spec = fetch_spec(&app).await;
    let version = operation(&spec, "version_info");
    let response = app
        .clone()
        .oneshot(request("GET", "/version", None, None))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let media = response.headers()["content-type"]
        .to_str()
        .unwrap()
        .to_owned();
    let live = axum::body::to_bytes(response.into_body(), BODY_CAP)
        .await
        .unwrap();
    let drift = |spec: &Value, body: &[u8]| {
        let observation = validate_response(spec, &version, 200, Some(&media), body);
        let report = report(spec, &[observation], &exclusions()).unwrap();
        findings_for(&report, "version_info")
    };
    assert!(
        drift(&spec, &live).is_empty(),
        "the unmutated control conforms"
    );

    // `#[utoipa::path]` without `body = VersionInfo`: no documented content.
    let mut no_content = spec.clone();
    no_content["paths"]["/version"]["get"]["responses"]["200"]
        .as_object_mut()
        .unwrap()
        .remove("content");
    assert!(drift(&no_content, &live).contains(&"unexpected-response-body".to_owned()));

    // A documented media type whose schema was dropped.
    let mut no_schema = spec.clone();
    no_schema["paths"]["/version"]["get"]["responses"]["200"]["content"]["application/json"]
        .as_object_mut()
        .unwrap()
        .remove("schema");
    assert!(drift(&no_schema, &live).contains(&"missing-response-schema".to_owned()));

    // `VersionInfo.name` renamed in the served contract only.
    let mut renamed_contract = spec.clone();
    let schema = &mut renamed_contract["components"]["schemas"]["VersionInfo"];
    let property = schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("name")
        .unwrap();
    schema["properties"]["product_name"] = property;
    for required in schema["required"].as_array_mut().unwrap() {
        if required == "name" {
            *required = json!("product_name");
        }
    }
    assert!(drift(&renamed_contract, &live).contains(&"response-schema-mismatch".to_owned()));

    // `name` renamed on the wire only: schema and contract decoding both fail.
    let mut body: Value = serde_json::from_slice(&live).unwrap();
    let name = body.as_object_mut().unwrap().remove("name").unwrap();
    body["product_name"] = name;
    let renamed_wire = serde_json::to_vec(&body).unwrap();
    let kinds = drift(&spec, &renamed_wire);
    assert!(
        kinds.contains(&"response-schema-mismatch".to_owned()),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&"contract-decode-failure".to_owned()),
        "{kinds:?}"
    );

    // An undocumented status is drift even with a conforming body.
    let observation = validate_response(&spec, &version, 299, Some(&media), &live);
    assert_eq!(observation.findings[0].kind, "undocumented-status");
}

#[tokio::test]
async fn the_report_fails_closed_on_missing_evidence_and_bad_exclusions() {
    let app = build_app(common::build_me_state(), &config());
    let spec = fetch_spec(&app).await;
    let empty = report(&spec, &[], &exclusions()).unwrap();
    assert_eq!(empty["complete"], false, "no observations never passes");
    assert_eq!(
        empty["openapi_runtime_drift_finding_count"],
        empty["findings"].as_array().unwrap().len()
    );
    assert!(
        findings_for(&empty, "version_info").contains(&"unreached-operation".to_owned()),
        "unobserved operations are counted"
    );

    let health = operation(&spec, "health_check");
    let entry = |status: Option<u16>| {
        json!({
            "method": health.method, "path": health.path,
            "operation_id": health.operation_id, "status": status, "reason": "control",
        })
    };
    for (invalid, why) in [
        (
            json!([{"method":"get","path":"/absent","operation_id":"invented","reason":"x"}]),
            "unknown operation",
        ),
        (json!([entry(None), entry(None)]), "duplicate"),
        (json!([entry(None), entry(Some(200))]), "redundant status"),
        (json!([entry(Some(404))]), "undocumented success status"),
        (
            json!([{"method": health.method, "path": health.path,
                    "operation_id": health.operation_id, "reason": " "}]),
            "blank reason",
        ),
        (json!([{"operation_id": "health_check"}]), "malformed"),
    ] {
        assert!(report(&spec, &[], &invalid).is_err(), "{why}");
    }

    let unknown = Observation {
        operation: Operation {
            method: "get".to_owned(),
            path: "/absent".to_owned(),
            operation_id: "invented".to_owned(),
        },
        status: 200,
        body_complete: true,
        findings: Vec::new(),
    };
    assert!(report(&spec, &[unknown], &exclusions()).is_err());
}

#[test]
fn every_exclusion_is_enumerated_in_the_crate_readme() {
    let readme = include_str!("../README.md");
    for exclusion in exclusions().as_array().unwrap() {
        let row = format!(
            "| `{} {}` (`{}`) |",
            exclusion["method"].as_str().unwrap().to_ascii_uppercase(),
            exclusion["path"].as_str().unwrap(),
            exclusion["operation_id"].as_str().unwrap(),
        );
        assert!(readme.contains(&row), "README exclusion table lacks {row}");
    }
}

// ── Router-wide failures ─────────────────────────────────────────────────────

#[tokio::test]
async fn every_protected_served_operation_has_a_live_problem_denial() {
    let (state, _queue) = create_state_with_queue().await;
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    for operation in inventory(&spec).unwrap().into_keys() {
        let security = &spec["paths"][&operation.path][&operation.method]["security"];
        if !security.as_array().is_some_and(|requirements| {
            requirements.iter().any(|requirement| {
                requirement
                    .as_object()
                    .is_some_and(|requirement| !requirement.is_empty())
            })
        }) {
            continue;
        }
        let mut uri = operation.path.clone();
        // Authentication runs before tenant or body extraction: no fixture can
        // manufacture scope or dispatch merely by supplying a path parameter.
        while let Some(start) = uri.find('{') {
            let end = uri[start..].find('}').unwrap() + start;
            uri.replace_range(start..=end, "contract-denial");
        }
        let method = operation.method.to_ascii_uppercase();
        let (_, problem) = exchange(
            &app,
            &spec,
            &operation.operation_id,
            request(&method, &uri, None, None),
            401,
        )
        .await;
        assert_eq!(problem["detail"], "Authentication required");
    }
}

#[tokio::test]
async fn global_rate_rejection_is_a_documented_problem_before_authentication() {
    let (state, _queue) = create_state_with_queue().await;
    let spec = fetch_spec(&build_app(state.clone(), &config())).await;
    let mut limited = ApiConfig::for_test();
    limited.rate_limit_per_second = 1;
    let app = build_app(state, &limited);
    let uri = ws_path("/workflows");
    exchange(
        &app,
        &spec,
        "list_workflows",
        request("GET", &uri, None, None),
        401,
    )
    .await;
    let (headers, problem) = exchange(
        &app,
        &spec,
        "list_workflows",
        request("GET", &uri, Some("intentionally-invalid-control"), None),
        429,
    )
    .await;
    assert_eq!(headers["retry-after"], "1");
    let problem: v1::problem::ProblemDetails = serde_json::from_value(problem).unwrap();
    assert_eq!(problem.type_uri, "https://nebula.dev/problems/rate-limit");
    // Probes stay outside the limiter.
    exchange(
        &app,
        &spec,
        "health_check",
        request("GET", "/health", None, None),
        200,
    )
    .await;
}

/// Body-limit and JSON-decoding rejections keep axum's status codes but are
/// documented RFC 9457 problems that never quote the submitted body.
#[tokio::test]
async fn request_body_rejections_are_documented_payload_free_problems() {
    let (state, _queue) = create_state_with_queue().await;
    let mut limited = config();
    limited.max_body_size = 256;
    let app = build_app(state, &limited);
    let spec = fetch_spec(&app).await;
    let jwt = create_test_jwt();
    let uri = ws_path("/workflows");
    let canary = "body-canary-".repeat(32);
    let raw = |content_type: Option<&str>, body: String| {
        let mut builder = Request::builder()
            .method("POST")
            .uri(&uri)
            .header("authorization", format!("Bearer {jwt}"));
        if let Some(content_type) = content_type {
            builder = builder.header("content-type", content_type);
        }
        builder.body(Body::from(body)).unwrap()
    };
    for (request, status) in [
        (
            raw(
                Some("application/json"),
                json!({"name": canary, "definition": {}}).to_string(),
            ),
            413,
        ),
        (raw(None, json!({"name": "body-canary"}).to_string()), 415),
        (
            raw(
                Some("application/json"),
                "{\"name\":\"body-canary".to_owned(),
            ),
            400,
        ),
        (
            raw(
                Some("application/json"),
                json!({"name": ["body-canary"], "definition": {}}).to_string(),
            ),
            422,
        ),
    ] {
        let (_, problem) = exchange(&app, &spec, "create_workflow", request, status).await;
        assert!(!problem.to_string().contains("body-canary"), "{problem}");
    }
}

/// An unavailable durable control queue is an honest, documented 503 on both
/// control routes; nothing is enqueued and nothing claims success.
#[tokio::test]
async fn control_queue_outage_is_a_documented_problem() {
    use nebula_storage_port::store::ExecutionStore;

    let (state, executions) = common::create_state_with_failing_queue().await;
    let execution_id = nebula_core::ExecutionId::new();
    let workflow_id = nebula_core::WorkflowId::new();
    ExecutionStore::create(
        &executions,
        &common::port_scope(),
        &execution_id.to_string(),
        &workflow_id.to_string(),
        json!({
            "workflow_id": workflow_id.to_string(),
            "status": "running",
            "started_at": chrono::Utc::now().timestamp(),
            "input": {},
        }),
    )
    .await
    .unwrap();
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    let jwt = create_test_jwt();
    let path = ws_path(&format!("/executions/{execution_id}"));
    exchange(
        &app,
        &spec,
        "terminate_execution",
        request("POST", &format!("{path}/terminate"), Some(&jwt), None),
        503,
    )
    .await;
    exchange(
        &app,
        &spec,
        "cancel_execution",
        request("DELETE", &path, Some(&jwt), None),
        503,
    )
    .await;
}

// ── Success branches ─────────────────────────────────────────────────────────

#[tokio::test]
async fn production_github_oauth_start_is_hermetic_and_matches_the_wire_contract() {
    use std::collections::HashMap;

    use nebula_api::{
        OAuthIdentityRuntime,
        config::{OAuthProviderConfig, OAuthProvidersConfig},
    };
    use secrecy::SecretString;

    let mut providers = HashMap::new();
    providers.insert(
        v1::auth::OAuthProvider::GitHub,
        OAuthProviderConfig {
            client_id: SecretString::new("fixture-client".into()),
            client_secret: SecretString::new("fixture-secret".into()),
        },
    );
    // The production fixed GitHub profile constructs its authorization URL
    // locally. Only the callback needs a provider-issued code and egress.
    let runtime = OAuthIdentityRuntime::from_config(OAuthProvidersConfig { providers })
        .unwrap()
        .unwrap();
    let backend = InMemoryAuthBackend::new().with_oauth_runtime(Arc::new(runtime));
    let state = common::build_me_state()
        .with_auth_backend(Arc::new(backend))
        .with_public_url("https://nebula.test");
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    let mut start = request("GET", "/api/v1/auth/oauth/github", None, None);
    start
        .headers_mut()
        .insert("host", "nebula.test".parse().unwrap());
    let (headers, body) = exchange(&app, &spec, "oauth_start", start, 200).await;
    assert!(headers.contains_key("set-cookie"));
    let decoded: v1::auth::OAuthStartResponse = serde_json::from_value(body).unwrap();
    assert!(
        decoded
            .authorize_url
            .starts_with("https://github.com/login/oauth/authorize?")
    );
    assert!(!decoded.state.is_empty());
}

#[tokio::test]
async fn shared_contract_workflow_crud_activation_and_start_round_trip() {
    let (state, _queue) = create_state_with_queue().await;
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    let jwt = create_test_jwt();
    for (id, path) in [
        ("health_check", "/health"),
        ("version_info", "/version"),
        ("readiness_check", "/ready"),
    ] {
        exchange(&app, &spec, id, request("GET", path, None, None), 200).await;
    }
    let create = v1::workflow::CreateWorkflowRequest {
        name: "OpenAPI runtime contract".to_owned(),
        description: None,
        definition: make_valid_workflow_definition(&nebula_core::WorkflowId::new()),
    };
    let (_, created) = exchange(
        &app,
        &spec,
        "create_workflow",
        request(
            "POST",
            &ws_path("/workflows"),
            Some(&jwt),
            Some(wire(create)),
        ),
        201,
    )
    .await;
    let created: v1::workflow::WorkflowResponse = serde_json::from_value(created).unwrap();
    let path = ws_path(&format!("/workflows/{}", created.id));
    exchange(
        &app,
        &spec,
        "get_workflow",
        request("GET", &path, Some(&jwt), None),
        200,
    )
    .await;
    exchange(
        &app,
        &spec,
        "list_workflows",
        request("GET", &ws_path("/workflows"), Some(&jwt), None),
        200,
    )
    .await;
    let update = v1::workflow::UpdateWorkflowRequest {
        name: Some("Changed through the contract".to_owned()),
        description: None,
        definition: None,
    };
    exchange(
        &app,
        &spec,
        "update_workflow",
        request("PUT", &path, Some(&jwt), Some(wire(update))),
        200,
    )
    .await;
    exchange(
        &app,
        &spec,
        "validate_workflow_handler",
        request("POST", &format!("{path}/validate"), Some(&jwt), None),
        200,
    )
    .await;
    exchange(
        &app,
        &spec,
        "activate_workflow",
        request("POST", &format!("{path}/activate"), Some(&jwt), None),
        200,
    )
    .await;
    for (id, suffix) in [
        ("execute_workflow", "execute"),
        ("start_execution", "executions"),
    ] {
        let start = v1::execution::StartExecutionRequest {
            input: Some(json!({})),
        };
        exchange(
            &app,
            &spec,
            id,
            request(
                "POST",
                &format!("{path}/{suffix}"),
                Some(&jwt),
                Some(wire(start)),
            ),
            202,
        )
        .await;
    }
    // A separate unactivated workflow verifies the empty documented 204 body.
    let create = v1::workflow::CreateWorkflowRequest {
        name: "Delete contract".to_owned(),
        description: None,
        definition: json!({}),
    };
    let (_, created) = exchange(
        &app,
        &spec,
        "create_workflow",
        request(
            "POST",
            &ws_path("/workflows"),
            Some(&jwt),
            Some(wire(create)),
        ),
        201,
    )
    .await;
    let created: v1::workflow::WorkflowResponse = serde_json::from_value(created).unwrap();
    exchange(
        &app,
        &spec,
        "delete_workflow",
        request(
            "DELETE",
            &ws_path(&format!("/workflows/{}", created.id)),
            Some(&jwt),
            None,
        ),
        204,
    )
    .await;
}

#[tokio::test]
async fn catalog_read_models_return_their_contract_types() {
    let (actions, plugins) = common::catalog_registries();
    let state = common::build_me_state()
        .with_action_registry(actions)
        .with_plugin_registry(plugins);
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    let jwt = create_test_jwt();
    let (_, listed) = exchange(
        &app,
        &spec,
        "list_actions",
        request("GET", "/api/v1/actions", Some(&jwt), None),
        200,
    )
    .await;
    let listed: v1::catalog::ListActionsResponse = serde_json::from_value(listed).unwrap();
    assert!(
        listed
            .actions
            .iter()
            .any(|action| action.key == "core.echo")
    );
    exchange(
        &app,
        &spec,
        "get_action",
        request("GET", "/api/v1/actions/core.echo", Some(&jwt), None),
        200,
    )
    .await;
    let (_, listed) = exchange(
        &app,
        &spec,
        "list_plugins",
        request("GET", "/api/v1/plugins", Some(&jwt), None),
        200,
    )
    .await;
    let listed: v1::catalog::ListPluginsResponse = serde_json::from_value(listed).unwrap();
    assert!(listed.plugins.iter().any(|plugin| plugin.key == "core"));
    exchange(
        &app,
        &spec,
        "get_plugin",
        request("GET", "/api/v1/plugins/core", Some(&jwt), None),
        200,
    )
    .await;
}

/// Signup through password reset, including MFA enrollment and the two-step
/// login it then requires, over cookie-bearing browser requests.
#[tokio::test]
async fn plane_a_identity_lifecycle_round_trips_the_auth_contract() {
    use nebula_api::domain::auth::backend::{CSRF_COOKIE, SESSION_COOKIE};

    let backend = Arc::new(InMemoryAuthBackend::new());
    let state: AppState =
        common::build_me_state().with_auth_backend(Arc::clone(&backend) as Arc<dyn AuthBackend>);
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    let email = "conformance@nebula.dev";
    let password = "conformance-password-1";
    let latest_token = || {
        backend
            .emails()
            .last()
            .map(|message| message.body.clone())
            .unwrap()
    };
    let session_of = |headers: &HeaderMap| {
        headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(|cookie| cookie.strip_prefix(&format!("{SESSION_COOKIE}=")))
            .and_then(|cookie| cookie.split(';').next())
            .map(str::to_owned)
            .unwrap()
    };
    let with_session = |method: &str, uri: &str, session: &str, csrf: &str, body: Option<Value>| {
        let mut request = request(method, uri, None, body);
        let headers = request.headers_mut();
        headers.insert(
            "cookie",
            format!("{SESSION_COOKIE}={session}; {CSRF_COOKIE}={csrf}")
                .parse()
                .unwrap(),
        );
        headers.insert("x-csrf-token", csrf.parse().unwrap());
        request
    };

    let signup = v1::auth::SignupRequest {
        email: email.to_owned(),
        password: v1::auth::SecretString::new(password.to_owned()),
        display_name: "Conformance".to_owned(),
    };
    let (_, signed_up) = exchange(
        &app,
        &spec,
        "signup",
        request("POST", "/api/v1/auth/signup", None, Some(wire(signup))),
        200,
    )
    .await;
    let signed_up: v1::auth::SignupResponse = serde_json::from_value(signed_up).unwrap();
    assert!(signed_up.verification_email_sent);
    let verify = v1::auth::VerifyEmailRequest {
        token: latest_token(),
    };
    exchange(
        &app,
        &spec,
        "verify_email",
        request(
            "POST",
            "/api/v1/auth/verify-email",
            None,
            Some(wire(verify)),
        ),
        200,
    )
    .await;

    let login = || v1::auth::LoginRequest {
        email: email.to_owned(),
        password: v1::auth::SecretString::new(password.to_owned()),
        totp: None,
    };
    let (headers, logged_in) = exchange(
        &app,
        &spec,
        "login",
        request("POST", "/api/v1/auth/login", None, Some(wire(login()))),
        200,
    )
    .await;
    let logged_in: v1::auth::LoginResponse = serde_json::from_value(logged_in).unwrap();
    let session = session_of(&headers);
    let csrf = logged_in.csrf_token.clone();

    let (_, enrolled) = exchange(
        &app,
        &spec,
        "mfa_enroll",
        with_session("POST", "/api/v1/auth/mfa/enroll", &session, &csrf, None),
        200,
    )
    .await;
    let enrolled: v1::auth::MfaEnrollResponse = serde_json::from_value(enrolled).unwrap();
    let code = || mfa::current_code(&enrolled.secret_base32).unwrap();
    let confirm = v1::auth::MfaConfirmEnrollRequest { code: code() };
    exchange(
        &app,
        &spec,
        "mfa_verify",
        with_session(
            "POST",
            "/api/v1/auth/mfa/verify",
            &session,
            &csrf,
            Some(wire(confirm)),
        ),
        200,
    )
    .await;
    exchange(
        &app,
        &spec,
        "logout",
        with_session("POST", "/api/v1/auth/logout", &session, &csrf, None),
        200,
    )
    .await;

    let (_, challenge) = exchange(
        &app,
        &spec,
        "login",
        request("POST", "/api/v1/auth/login", None, Some(wire(login()))),
        202,
    )
    .await;
    let challenge: v1::auth::MfaChallengeResponse = serde_json::from_value(challenge).unwrap();
    let complete = v1::auth::MfaLoginCompleteRequest {
        code: code(),
        challenge_token: challenge.challenge_token.clone(),
    };
    exchange(
        &app,
        &spec,
        "mfa_complete_login",
        request("POST", "/api/v1/auth/login/mfa", None, Some(wire(complete))),
        200,
    )
    .await;

    let forgot = v1::auth::ForgotPasswordRequest {
        email: email.to_owned(),
    };
    exchange(
        &app,
        &spec,
        "forgot_password",
        request(
            "POST",
            "/api/v1/auth/forgot-password",
            None,
            Some(wire(forgot)),
        ),
        202,
    )
    .await;
    let reset = v1::auth::ResetPasswordRequest {
        token: latest_token(),
        new_password: v1::auth::SecretString::new("conformance-password-2".to_owned()),
    };
    exchange(
        &app,
        &spec,
        "reset_password",
        request(
            "POST",
            "/api/v1/auth/reset-password",
            None,
            Some(wire(reset)),
        ),
        200,
    )
    .await;
}

#[tokio::test]
async fn workspace_grant_upsert_and_revocation_round_trip() {
    use common::org_support::{OrgActor, create_org_state, seed_member};

    let (state, store, admin) = create_org_state();
    let member = OrgActor::new_user();
    seed_member(
        &store,
        member.principal.clone(),
        nebula_core::OrgRole::OrgMember,
    )
    .await;
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    let uri = format!(
        "/api/v1/orgs/{TEST_ORG}/workspaces/{TEST_WS}/members/{}",
        member.user_id
    );
    let grant = v1::workspace_membership::UpsertWorkspaceMemberRequest {
        role: v1::shared::WorkspaceRoleDto("runner".to_owned()),
    };
    exchange(
        &app,
        &spec,
        "upsert_workspace_member",
        request("PUT", &uri, Some(&admin.jwt), Some(wire(grant))),
        200,
    )
    .await;
    exchange(
        &app,
        &spec,
        "remove_workspace_member",
        request("DELETE", &uri, Some(&admin.jwt), None),
        200,
    )
    .await;
}

/// A testable, revocable credential type, so the `test` and `revoke`
/// capability routes reach their success branch.
mod probe_credential {
    use nebula_credential::{
        CredentialContext, CredentialMetadataDraft, SecretString,
        error::CredentialError,
        resolve::{StaticResolveResult, TestResult},
        scheme::SecretToken,
    };
    use nebula_schema::Schema;
    use serde::Deserialize;

    /// Create-form properties.
    #[derive(Schema, Deserialize)]
    pub(super) struct ProbeProperties {
        /// Probe token.
        #[field(secret, label = "Token")]
        #[validate(required)]
        token: SecretString,
    }

    pub(super) struct ProbeCredential;

    #[nebula_credential::credential(key = "conformance_probe")]
    impl ProbeCredential {
        type Properties = ProbeProperties;
        type Scheme = SecretToken;
        type State = SecretToken;

        fn metadata() -> CredentialMetadataDraft {
            CredentialMetadataDraft::new(
                nebula_core::credential_key!("conformance_probe"),
                nebula_credential::metadata_name!("Conformance Probe"),
                "testable and revocable credential for the OpenAPI conformance suite",
            )
        }

        fn project(state: &SecretToken) -> SecretToken {
            state.clone()
        }

        async fn resolve(
            properties: &ProbeProperties,
            _ctx: &CredentialContext,
        ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
            Ok(StaticResolveResult::Complete(SecretToken::new(
                properties.token.clone(),
            )))
        }

        async fn test(
            _scheme: &SecretToken,
            _ctx: &CredentialContext,
        ) -> Result<TestResult, CredentialError> {
            Ok(TestResult::Success)
        }

        async fn revoke(
            _state: &mut SecretToken,
            _ctx: &CredentialContext,
        ) -> Result<(), CredentialError> {
            Ok(())
        }
    }
}

#[tokio::test]
async fn credential_test_and_revoke_capabilities_round_trip() {
    use nebula_credential::{
        CredentialRegistry, DispatchOps, ErasedPendingStore, register_revocable_ops,
        register_runtime_ops, register_testable_ops,
    };
    use probe_credential::ProbeCredential;

    let mut registry = CredentialRegistry::new();
    registry
        .register(ProbeCredential, "nebula-api-test")
        .unwrap();
    let mut ops = DispatchOps::<ErasedPendingStore>::new();
    register_runtime_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).unwrap();
    register_testable_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).unwrap();
    register_revocable_ops::<ProbeCredential, ErasedPendingStore>(&mut ops).unwrap();
    let key = Arc::new(
        nebula_storage::credential::EnvKeyProvider::from_base64(common::TEST_CRED_KEY_B64).unwrap(),
    );
    let service =
        nebula_api::ports::credential_service_factory::with_memory_store_parts(key, registry, ops)
            .await
            .unwrap();
    let (state, _queue) = create_state_with_queue().await;
    let state = state.with_credential_gateway(
        nebula_api::ports::credential_command::test_gateway_from_service(service),
    );
    let app = build_app(state, &config());
    let spec = fetch_spec(&app).await;
    let jwt = create_test_jwt();

    let create = v1::credential::CreateCredentialRequest {
        credential_key: "conformance_probe".to_owned(),
        name: "Conformance probe".to_owned(),
        description: None,
        data: json!({ "token": "probe-token" }),
        tags: None,
    };
    let (_, created) = exchange(
        &app,
        &spec,
        "create_credential",
        request(
            "POST",
            &ws_path("/credentials"),
            Some(&jwt),
            Some(wire(create)),
        ),
        200,
    )
    .await;
    let created: v1::credential::CredentialResponse = serde_json::from_value(created).unwrap();
    let path = ws_path(&format!("/credentials/{}", created.id));
    exchange(
        &app,
        &spec,
        "test_credential",
        request("POST", &format!("{path}/test"), Some(&jwt), None),
        200,
    )
    .await;
    exchange(
        &app,
        &spec,
        "revoke_credential",
        request("POST", &format!("{path}/revoke"), Some(&jwt), None),
        200,
    )
    .await;
}

// ── Producer aggregation ─────────────────────────────────────────────────────

/// Run only after the producer has completed every API fixture suite. This
/// final step consumes their actual responses, writes the artifact, and then
/// fails on any drift finding.
#[test]
#[ignore = "producer aggregation requires a fresh completed observation directory"]
fn emit_openapi_runtime_compatibility_report() {
    let directory =
        std::env::var_os("NEBULA_OPENAPI_OBSERVATIONS").expect("producer observation directory");
    let destination = std::env::var_os("NEBULA_OPENAPI_REPORT").expect("producer output path");
    let entries: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        entries.len() <= 20_000,
        "bounded conformance observation collection"
    );
    let mut spec = None;
    let mut observations = Vec::new();
    for entry in entries {
        assert!(
            entry.file_type().unwrap().is_file(),
            "only regular observation files"
        );
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        let bytes = fs::read(entry.path()).unwrap();
        let json = std::path::Path::new(name)
            .extension()
            .is_some_and(|extension| extension == "json");
        if json && name.starts_with("spec-") {
            assert!(bytes.len() <= BODY_CAP);
            let observed: Value = serde_json::from_slice(&bytes).unwrap();
            if let Some(spec) = &spec {
                assert_eq!(
                    spec, &observed,
                    "served spec must remain identical across fixture states"
                );
            } else {
                spec = Some(observed);
            }
        } else if json && name.starts_with("case-") {
            assert!(bytes.len() <= 16 * 1024);
            let observation: Observation = serde_json::from_slice(&bytes).unwrap();
            observations.push(observation);
        } else {
            panic!("unknown producer observation file");
        }
    }
    let output = report(
        &spec.expect("actual served spec"),
        &observations,
        &exclusions(),
    )
    .unwrap();
    fs::write(destination, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    assert_eq!(
        output["openapi_runtime_drift_finding_count"], 0,
        "NS15 drift findings: {output}"
    );
}
