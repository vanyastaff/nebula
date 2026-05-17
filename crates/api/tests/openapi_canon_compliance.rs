//! Canon §4.5 stub-honesty gate (M3.2 Task 6.5).
//!
//! Locks the OpenAPI document against the failure mode where a stub
//! handler is silently advertised as a fully-shipped endpoint.
//!
//! **Static check** — every operation flagged `deprecated = true` MUST
//! declare a `501` response (ADR-0047 Stub Endpoint Policy).
//!
//! **Runtime check** — every honest-501 stub endpoint is probed against a
//! booted in-memory app. The remaining inventory is the org-record
//! (`get`/`update`/`delete_org`), service-account
//! (`list`/`create`/`delete_service_account`), and `execution::restart`
//! handlers.
//!
//! Graduated stub→implemented (removed from this inventory):
//! `execution::terminate` via the durable control queue (ADR-0008 A3 /
//! ADR-0016, covered by `execution_terminate_e2e.rs`); the five `me/*`
//! profile/PAT endpoints (`get_me`, `update_me`, `list_my_tokens`,
//! `create_token`, `delete_token`) via the Plane-A `AuthBackend` port
//! (covered by `me_e2e.rs`); — Phase 3 — `me::list_my_orgs` plus the
//! three org member endpoints (`org::list_members`, `org::add_member`,
//! `org::remove_member`) via the shared `MembershipStore` (canon §4.5
//! "Option 1" honest contract — the fake email-invitation shape was
//! dropped for direct add-by-principal; covered, incl. abuse cases, by
//! `org_e2e.rs`); and the resource catalog surface
//! (`resource::{list,get,create,update,delete}_resource` +
//! `resource::get_resource_status`) — config-CRUD + CAS + a read-only
//! runtime-status projection (ADR-0067; covered by
//! `resource_handlers.rs`). The org-record + service-account endpoints
//! stay honest-501 (no org-record store; no end-to-end
//! `Principal::ServiceAccount` auth path).
//!
//! The accepted runtime outcomes are **501** (the
//! `ApiError::NotImplemented` variant) for stubs that reach the handler
//! body and **403** (`ApiError::InsufficientRole` / `Forbidden`) for
//! stubs whose RBAC gate (`tenant.require(...)`) runs before the handler.
//! Both 403 and 501 are explicitly advertised in the spec for RBAC-gated
//! stubs (see ADR-0047 §4 Stub Endpoint Policy + the per-handler
//! `responses(...)` blocks). The legacy 500 outcome (when the stub
//! returned `ApiError::Internal("not implemented")`) is no longer
//! accepted — that deviation closed when stubs migrated to
//! `ApiError::NotImplemented`.

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
         after `execution::terminate` and the resource catalog graduated \
         stub→implemented the remaining stubs are under org/ (org-record + \
         service-account) and execution/restart"
    );
}

/// Every stub mounted today, keyed by the M3.2 audit. Methods that take
/// a JSON body get the empty `{}` payload — every stub handler accepts
/// `Json<serde_json::Value>`, so an empty object satisfies extraction
/// without tripping into 400 Bad Request before the handler runs.
fn stub_endpoints() -> Vec<(&'static str, String, Option<&'static str>)> {
    let sa_id = "sa_00000000000000000000000001";
    let _exec_id = "exe_00000000000000000000000001";
    vec![
        // me/* — all six graduated (5 via the Plane-A `AuthBackend` port;
        // `list_my_orgs` via the shared `MembershipStore` in Phase 3).
        // None remain in this inventory.
        //
        // orgs/* — 6 honest-501 stubs remain. The 3 member endpoints
        // (`list_members`/`add_member`/`remove_member`) graduated
        // stub→implemented in Phase 3 against the shared `MembershipStore`
        // ("Option 1" honest contract — covered, incl. abuse cases, by
        // `org_e2e.rs`). The org-record + service-account endpoints stay
        // honest-501 (no org-record store; no end-to-end
        // `Principal::ServiceAccount` auth path).
        ("GET", format!("/api/v1/orgs/{TEST_ORG}"), None),
        ("PATCH", format!("/api/v1/orgs/{TEST_ORG}"), Some("{}")),
        ("DELETE", format!("/api/v1/orgs/{TEST_ORG}"), None),
        (
            "GET",
            format!("/api/v1/orgs/{TEST_ORG}/service-accounts"),
            None,
        ),
        (
            "POST",
            format!("/api/v1/orgs/{TEST_ORG}/service-accounts"),
            Some("{}"),
        ),
        (
            "DELETE",
            format!("/api/v1/orgs/{TEST_ORG}/service-accounts/{sa_id}"),
            None,
        ),
        // resource — 0 stubs (the whole resource catalog surface
        // graduated stub→implemented: config-CRUD + CAS + a read-only
        // runtime-status projection, ADR-0067; covered end-to-end by
        // resource_handlers.rs).
        // execution — 1 stub (terminate graduated stub→implemented:
        // real §12.2 durable-control-queue endpoint, ADR-0008 A3 /
        // ADR-0016; covered end-to-end by execution_terminate_e2e.rs).
        (
            "POST",
            format!("/api/v1/orgs/{TEST_ORG}/workspaces/{TEST_WS}/executions/{_exec_id}/restart"),
            None,
        ),
    ]
}

#[tokio::test]
async fn stub_endpoints_return_501_at_runtime() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = build_app(state, &config);

    let token = create_test_jwt();
    let stubs = stub_endpoints();
    assert_eq!(
        stubs.len(),
        7,
        "stub coverage list must enumerate all remaining audit class-(c) \
         endpoints. The M3.2 audit identified 18. Graduated \
         stub→implemented (removed from this inventory): \
         `execution::terminate` (ADR-0008 A3 / ADR-0016 — \
         execution_terminate_e2e.rs); the five `me/*` profile/PAT \
         endpoints (Plane-A `AuthBackend` port — me_e2e.rs); **Phase \
         3** `me::list_my_orgs` + the three org member endpoints \
         (`list_members`/`add_member`/`remove_member`) via the shared \
         `MembershipStore` (org_e2e.rs); and the resource catalog surface \
         (config-CRUD + CAS + read-only status, ADR-0067 — \
         resource_handlers.rs). That leaves 7 honest-501: 3 org-record \
         (`get`/`update`/`delete_org`) + 3 service-account + execution \
         restart; got {}",
        stubs.len()
    );

    for (method, path, body) in &stubs {
        let mut req = Request::builder()
            .method(*method)
            .uri(path)
            .header("Authorization", format!("Bearer {token}"))
            .header("Cookie", TEST_CSRF_COOKIE)
            .header("X-CSRF-Token", "test-csrf-token");
        let request_body = match body {
            Some(payload) => {
                req = req.header("Content-Type", "application/json");
                Body::from(*payload)
            },
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(req.body(request_body).unwrap())
            .await
            .expect("stub endpoint must respond");

        let status = response.status().as_u16();
        assert!(
            status == 501 || status == 403,
            "Stub endpoint {method} {path} expected 501 (per ADR-0047 Stub \
             Endpoint Policy + the `ApiError::NotImplemented` variant) or \
             403 (per the RBAC gate, when the handler carries one); got \
             {status}. Either the stub got accidentally implemented (drop \
             the `#[deprecated]` + 501 response declaration) or auth/tenancy \
             middleware rejected the request with a non-RBAC failure — \
             verify the test harness reaches the handler body."
        );

        // Defense-in-depth: an `ApiError::NotImplemented` response goes
        // through `IntoResponse for ApiError` which sets `Content-Type:
        // application/problem+json` per RFC 9457. A 501 emitted by
        // middleware (or a generic axum error path) would carry a
        // different content-type — this assertion confirms the request
        // actually reached the handler and the canon §4.5 honesty
        // contract was exercised end-to-end.
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
             handler."
        );
    }
}
