//! Canon §4.5 stub-honesty gate (M3.2 Task 6.5).
//!
//! Locks the OpenAPI document against the failure mode where a stub
//! handler is silently advertised as a fully-shipped endpoint:
//!
//! - **Static check** — every operation flagged `deprecated = true` MUST
//!   declare a `501` response. ADR-0047 Stub Endpoint Policy.
//! - **Runtime check** — a curated subset of stub endpoints is hit on a
//!   booted in-memory app and asserted to return either `500` (current
//!   `ApiError::Internal("not implemented")` baseline) or `501`. The
//!   transition from 500 → 501 is a single follow-up PR (`Internal` →
//!   `NotImplemented`) tracked by the Plane-A backend extension milestone;
//!   the test accepts both so that follow-up does not silently slip.

mod common;

use axum::{
    body::Body,
    http::{Request, header::CONTENT_TYPE},
};
use nebula_api::{ApiConfig, build_app};
use serde_json::Value;
use tower::ServiceExt;

use crate::common::{
    TEST_CSRF_COOKIE, TEST_ORG, TEST_WS, create_state_with_queue, create_test_jwt,
};

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "options", "head", "trace",
];

async fn fetch_spec(app: &axum::Router) -> Value {
    let response = app
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
    assert_eq!(response.status(), 200, "openapi.json must return 200");

    let body = axum::body::to_bytes(response.into_body(), 8 * 1_000_000)
        .await
        .expect("body must be readable");
    serde_json::from_slice(&body).expect("openapi.json body must be valid JSON")
}

#[tokio::test]
async fn deprecated_operations_must_advertise_501_response() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = build_app(state, &config);

    let spec = fetch_spec(&app).await;
    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .expect("spec must carry a paths object");

    let mut deprecated_count = 0_usize;
    for (path, operations) in paths {
        let ops = operations
            .as_object()
            .expect("each path entry must be an object");
        for (method, op) in ops {
            if !HTTP_METHODS.contains(&method.as_str()) {
                continue;
            }
            let deprecated = op
                .get("deprecated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !deprecated {
                continue;
            }
            deprecated_count += 1;

            let responses = op
                .get("responses")
                .and_then(Value::as_object)
                .unwrap_or_else(|| {
                    panic!("Deprecated operation {method} {path} has no responses object")
                });
            assert!(
                responses.contains_key("501"),
                "Deprecated operation {method} {path} MUST declare a 501 response per ADR-0047 \
                 Stub Endpoint Policy; got responses: {:?}",
                responses.keys().collect::<Vec<_>>()
            );
        }
    }

    assert!(
        deprecated_count > 0,
        "Spec must contain at least one deprecated stub operation; \
         the M3.2 audit identifies 18+ stub endpoints under me/, org/, \
         resource/, and execution/{{terminate,restart}}"
    );
}

#[tokio::test]
async fn stub_endpoints_return_500_or_501_at_runtime() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = build_app(state, &config);

    let token = create_test_jwt();

    // One representative endpoint per stub module per the M3.2 audit.
    // Tenant-scoped paths (`/orgs/{org}/...`) need the test-org / -ws
    // resolved by the resolvers wired in `create_state_with_queue`.
    let stubs: &[(&str, String)] = &[
        ("GET", "/api/v1/me".to_owned()),
        ("GET", format!("/api/v1/orgs/{TEST_ORG}")),
        (
            "GET",
            format!("/api/v1/orgs/{TEST_ORG}/workspaces/{TEST_WS}/resources"),
        ),
    ];

    for (method, path) in stubs {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(*method)
                    .uri(path)
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Cookie", TEST_CSRF_COOKIE)
                    .header("X-CSRF-Token", "test-csrf-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("stub endpoint must respond");

        let status = response.status().as_u16();
        assert!(
            status == 500 || status == 501,
            "Stub endpoint {method} {path} expected 500 (current `ApiError::Internal` baseline) \
             or 501 (post-conversion target); got {status}. Either the stub got accidentally \
             implemented (drop the #[deprecated] + 501 response declaration) or auth/tenancy \
             middleware rejected the request — verify the test harness wires both."
        );

        // Defense-in-depth: an `ApiError::Internal`/`NotImplemented`
        // response goes through `IntoResponse for ApiError` which sets
        // `Content-Type: application/problem+json` per RFC 9457. Any
        // 500/501 emitted by middleware (auth/tenancy) instead of the
        // stub handler would carry a different content-type — this
        // assertion confirms the request actually reached the handler
        // and the canon §4.5 honesty contract was exercised end-to-end.
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            content_type.contains("application/problem+json"),
            "Stub endpoint {method} {path} returned status {status} but \
             Content-Type was `{content_type}` — expected \
             `application/problem+json` (RFC 9457). A non-problem+json \
             response means the request was short-circuited by middleware \
             (auth/tenancy), so this test is not actually probing the \
             handler. Verify the test harness reaches the stub body."
        );
    }
}
