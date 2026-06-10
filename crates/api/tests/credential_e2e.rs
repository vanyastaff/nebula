//! Phase 4 — credential CRUD end-to-end + abuse-case coverage.
//!
//! Credentials are the highest-value target in the API surface, so this
//! file is deliberately heavy on the security cases (integration seam, credential secrecy /
//! STYLE §6):
//!
//! - Happy-path CRUD round-trips through the real axum `Router` over the
//!   wired in-memory credential store.
//! - **Secret never returned** — `get` / `list` response bodies are
//!   asserted to contain no plaintext.
//! - **Secret never in errors** — forced-error `ProblemDetails` bodies
//!   are asserted clean.
//! - **Secret never in logs** — the `assert_no_secret_in_logs` pattern
//!   (mirrored from `crates/credential/tests/redaction.rs`) wraps a full
//!   create→get→list→delete cycle on the single-threaded path.
//! - Cross-workspace isolation / IDOR — a credential created in ws-A is a
//!   flat 404 from ws-B (no existence disclosure).
//! - Honest endpoints (`test` / `refresh` / `revoke` / `resolve` /
//!   `continue` / type discovery) assert the honest 503.
//!
//! The shared harness (`create_state_with_queue`, `ws_path`,
//! `create_test_jwt`, `TEST_CSRF_*`) comes from `tests/common` — Phase
//! 1/2 precedent; no fixtures are duplicated here.

mod common;

use std::{
    future::Future,
    io::{self, Write},
    sync::{Arc, Mutex},
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    TEST_CSRF_COOKIE, TEST_CSRF_TOKEN, create_state_with_queue, create_test_jwt, ws_path,
};
use nebula_api::{ApiConfig, app};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

/// A secret value that must never surface in any response, error, or log.
const SECRET_TOKEN: &str = "sk-phase4-NEVER-LEAK-0xC0FFEE-abcdef0123456789";
const OTHER_WS: &str = "ws_00000000000000000000000002";

// ── Log-capture helper (mirrors crates/credential/tests/redaction.rs) ─────────

#[derive(Clone, Default)]
struct CaptureBuf(Arc<Mutex<Vec<u8>>>);

impl CaptureBuf {
    fn as_string(&self) -> String {
        let guard = self.0.lock().expect("capture buffer poisoned");
        String::from_utf8_lossy(&guard).into_owned()
    }
}

impl Write for CaptureBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self.0.lock().expect("capture buffer poisoned");
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureBuf {
    type Writer = CaptureBuf;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Run `body` with a capturing subscriber installed on the current
/// thread, then assert `forbidden` never leaked into any captured
/// event. Same contract as the credential-crate helper: the assertion
/// failure is the only signal; do not spawn inside `body`.
async fn assert_no_secret_in_logs<F, Fut>(forbidden: &str, body: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    assert!(
        !forbidden.is_empty(),
        "forbidden substring must be non-empty"
    );

    let buf = CaptureBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();

    {
        let _guard = tracing::subscriber::set_default(subscriber);
        body().await;
    }

    let captured = buf.as_string();
    assert!(
        !captured.contains(forbidden),
        "log-redaction invariant violated: secret {forbidden:?} leaked \
         into captured tracing output.\n---- captured ----\n{captured}\n\
         ------------------"
    );
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

fn auth_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE)
        .body(Body::empty())
        .unwrap()
}

fn auth_json(method: &str, uri: &str, token: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE)
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn create_body() -> serde_json::Value {
    serde_json::json!({
        "credential_key": "api_key",
        "name": "Phase4 Key",
        "description": "e2e credential",
        "data": { "api_key": SECRET_TOKEN, "server": "https://api.example.com" },
        "tags": { "env": "test" }
    })
}

// ── Happy-path CRUD ───────────────────────────────────────────────────────────

#[tokio::test]
async fn credential_crud_round_trips_and_never_echoes_secret() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    // CREATE
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials"),
            &token,
            &create_body(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "create must return 200");
    let created_body = body_string(resp).await;
    assert!(
        !created_body.contains(SECRET_TOKEN),
        "create response must not echo the secret: {created_body}"
    );
    let created: serde_json::Value = serde_json::from_str(&created_body).unwrap();
    let cred_id = created["id"].as_str().expect("created id").to_owned();
    assert!(cred_id.starts_with("cred_"), "id must be cred_<ULID>");
    assert_eq!(created["credential_key"], "api_key");
    assert_eq!(created["name"], "Phase4 Key");
    assert_eq!(created["auth_pattern"], "SecretToken");
    assert_eq!(created["version"], 1);
    assert_eq!(created["tags"]["env"], "test");

    // GET — metadata only, no secret
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_get(
            &ws_path(&format!("/credentials/{cred_id}")),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let got = body_string(resp).await;
    assert!(
        !got.contains(SECRET_TOKEN),
        "get response leaked the secret: {got}"
    );
    let got_json: serde_json::Value = serde_json::from_str(&got).unwrap();
    assert_eq!(got_json["id"], cred_id);
    assert!(
        got_json.get("data").is_none(),
        "get response must have no `data` field at all"
    );

    // LIST — summaries only, no secret
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_get(&ws_path("/credentials"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_string(resp).await;
    assert!(
        !listed.contains(SECRET_TOKEN),
        "list response leaked the secret: {listed}"
    );
    let listed_json: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed_json["total"], 1);
    assert_eq!(listed_json["credentials"][0]["id"], cred_id);
    assert!(
        listed_json["credentials"][0].get("data").is_none(),
        "list summary must have no `data` field"
    );

    // UPDATE (name only, optimistic version) — still no secret echoed
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "PUT",
            &ws_path(&format!("/credentials/{cred_id}")),
            &token,
            &serde_json::json!({ "name": "Renamed Phase4 Key", "version": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "update must return 200");
    let updated = body_string(resp).await;
    assert!(!updated.contains(SECRET_TOKEN), "update leaked: {updated}");
    let updated_json: serde_json::Value = serde_json::from_str(&updated).unwrap();
    assert_eq!(updated_json["name"], "Renamed Phase4 Key");
    assert_eq!(updated_json["version"], 2, "CAS update bumps version");

    // DELETE
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(ws_path(&format!("/credentials/{cred_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "delete must return 200");

    // GET after delete → 404, no secret in the problem body
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_get(
            &ws_path(&format!("/credentials/{cred_id}")),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let problem = body_string(resp).await;
    assert!(
        !problem.contains(SECRET_TOKEN),
        "404 problem body leaked the secret: {problem}"
    );
}

// ── Abuse: cross-workspace / IDOR isolation ───────────────────────────────────

#[tokio::test]
async fn credential_unknown_id_is_404_without_existence_disclosure() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    // A syntactically valid but never-created id (the IDOR / cross-ws
    // probe — the in-memory store is global, so an unknown id models a
    // credential that does not belong to this caller's scope).
    let unknown = "cred_00000000000000000000000099";

    for (method, path) in [
        ("GET", format!("/credentials/{unknown}")),
        ("DELETE", format!("/credentials/{unknown}")),
    ] {
        let app = app::build_app(state.clone(), &config);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(ws_path(&path))
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} unknown credential must be a flat 404"
        );
        let problem = body_string(resp).await;
        // No existence disclosure: a plain "not found", no "exists in
        // another workspace", no secret.
        assert!(
            !problem.contains(SECRET_TOKEN),
            "404 problem must not carry a secret"
        );
        assert!(
            !problem.to_lowercase().contains("another workspace"),
            "404 must not disclose cross-workspace existence: {problem}"
        );
    }

    // PUT with a CAS version on a non-existent credential → 404
    // (NotFound), still no secret echoed even though the request body
    // carries one.
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "PUT",
            &ws_path(&format!("/credentials/{unknown}")),
            &token,
            &serde_json::json!({ "data": { "api_key": SECRET_TOKEN }, "version": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let problem = body_string(resp).await;
    assert!(
        !problem.contains(SECRET_TOKEN),
        "update-on-missing problem body leaked the request secret: {problem}"
    );
}

// ── Abuse: optimistic-concurrency conflict (409), no secret leak ──────────────

#[tokio::test]
async fn credential_stale_version_conflicts_409_without_secret() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials"),
            &token,
            &create_body(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
    let cred_id = created["id"].as_str().unwrap().to_owned();

    // Stale CAS: expected version 99 (actual is 1) → 409.
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "PUT",
            &ws_path(&format!("/credentials/{cred_id}")),
            &token,
            &serde_json::json!({
                "data": { "api_key": SECRET_TOKEN },
                "version": 99
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "stale CAS version must return 409"
    );
    let problem = body_string(resp).await;
    assert!(
        !problem.contains(SECRET_TOKEN),
        "409 conflict body leaked the request secret: {problem}"
    );
}

#[tokio::test]
async fn credential_created_in_one_workspace_is_404_from_another_workspace() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "scoped credential",
        "data": { "api_key": SECRET_TOKEN }
    });
    let app = app::build_app(state.clone(), &config);
    let created = app
        .oneshot(auth_json("POST", &ws_path("/credentials"), &token, &body))
        .await
        .unwrap();
    assert!(
        created.status().is_success(),
        "credential create must succeed before cross-workspace probe"
    );

    let created_body: serde_json::Value =
        serde_json::from_str(&body_string(created).await).expect("created credential json");
    let id = created_body["id"].as_str().expect("created credential id");
    let other_ws_path = format!(
        "/api/v1/orgs/{}/workspaces/{OTHER_WS}/credentials/{id}",
        common::TEST_ORG
    );

    let app = app::build_app(state, &config);
    let probe = app.oneshot(auth_get(&other_ws_path, &token)).await.unwrap();

    assert_eq!(
        probe.status(),
        StatusCode::NOT_FOUND,
        "credential IDs must not resolve across workspace scopes"
    );
    let probe_body = body_string(probe).await;
    let problem: serde_json::Value = serde_json::from_str(&probe_body).expect("404 problem json");
    assert_eq!(problem["title"], "Not Found");
    assert_eq!(problem["detail"], "credential not found");
    let problem_obj = problem.as_object().expect("problem object");
    assert!(
        !problem_obj.contains_key("id"),
        "404 problem must not expose a credential id: {probe_body}"
    );
    assert!(
        !problem_obj.contains_key("name"),
        "404 problem must not expose a credential name: {probe_body}"
    );
    for forbidden in [
        id,
        "scoped credential",
        common::TEST_ORG,
        OTHER_WS,
        SECRET_TOKEN,
        "another workspace",
    ] {
        assert!(
            !probe_body.contains(forbidden),
            "cross-workspace 404 disclosed {forbidden:?}: {probe_body}"
        );
    }
}

// ── Abuse: secret never reaches logs across the full lifecycle ────────────────

#[tokio::test]
async fn credential_secret_never_appears_in_logs_across_lifecycle() {
    assert_no_secret_in_logs(SECRET_TOKEN, || async {
        let (state, _q) = create_state_with_queue().await;
        let config = ApiConfig::for_test();
        let token = create_test_jwt();

        // CREATE (request body carries the secret).
        let app = app::build_app(state.clone(), &config);
        let resp = app
            .oneshot(auth_json(
                "POST",
                &ws_path("/credentials"),
                &token,
                &create_body(),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let created: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
        let cred_id = created["id"].as_str().unwrap().to_owned();

        // GET / LIST / UPDATE(data) / DELETE — every path that touches
        // the secret blob must keep it out of tracing output.
        let app = app::build_app(state.clone(), &config);
        let _ = app
            .oneshot(auth_get(
                &ws_path(&format!("/credentials/{cred_id}")),
                &token,
            ))
            .await
            .unwrap();

        let app = app::build_app(state.clone(), &config);
        let _ = app
            .oneshot(auth_get(&ws_path("/credentials"), &token))
            .await
            .unwrap();

        let app = app::build_app(state.clone(), &config);
        let _ = app
            .oneshot(auth_json(
                "PUT",
                &ws_path(&format!("/credentials/{cred_id}")),
                &token,
                &serde_json::json!({ "data": { "api_key": SECRET_TOKEN } }),
            ))
            .await
            .unwrap();

        let app = app::build_app(state.clone(), &config);
        let _ = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(ws_path(&format!("/credentials/{cred_id}")))
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
    })
    .await;
}

// ── Abuse: validation rejects malformed input before persistence ──────────────

#[tokio::test]
async fn credential_create_rejects_non_object_data_400() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials"),
            &token,
            &serde_json::json!({
                "credential_key": "api_key",
                "name": "bad",
                "data": "not-an-object"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "non-object data must be rejected with 400 before persistence"
    );
}

#[tokio::test]
async fn credential_unauthenticated_request_is_401() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();

    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/credentials"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "missing bearer token must yield 401"
    );
}

// ── Lifecycle / acquisition reach the wired facade ────────────────────────────

/// The lifecycle and acquisition routes reach the handler and the wired
/// `CredentialService` answers honestly:
///
/// - `test`/`refresh`/`revoke` on a static `api_key` row → **400**
///   (capability gate: the type implements none of Testable /
///   Refreshable / Revocable) — never a faked success, never a 503.
/// - `resolve` with a static type completes synchronously (**200**,
///   `status: "complete"`, persisted id) — the route-shadow regression
///   guard: the literal `resolve` segment must reach the handler, not
///   404 in tenancy ULID parsing.
/// - `resolve/continue` on a non-interactive type → **400** (capability
///   gate), again proving handler reachability.
///
/// Caller-submitted secret material must never echo in any error body.
#[tokio::test]
async fn credential_lifecycle_and_acquisition_answer_through_the_facade() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a real credential so test/refresh/revoke reach the capability
    // gate on an existing row (not a pre-handler 404).
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials"),
            &token,
            &create_body(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "seed create must succeed");
    let created: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
    let cred_id = created["id"].as_str().unwrap().to_owned();

    // `test` / `refresh` / `revoke` on the static api_key type → 400
    // from the facade capability gate (closure absence IS capability
    // absence), with a secret-free problem body.
    let capability_400: &[(&str, String, serde_json::Value)] = &[
        (
            "POST",
            ws_path(&format!("/credentials/{cred_id}/test")),
            serde_json::json!({}),
        ),
        (
            "POST",
            ws_path(&format!("/credentials/{cred_id}/refresh")),
            serde_json::json!({}),
        ),
        (
            "POST",
            ws_path(&format!("/credentials/{cred_id}/revoke")),
            serde_json::json!({}),
        ),
    ];

    for (method, path, body) in capability_400 {
        let app = app::build_app(state.clone(), &config);
        let resp = app
            .oneshot(auth_json(method, path, &token, body))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{method} {path}: a static type must fail the capability gate \
             with 400 — never a faked success, never a stub 503"
        );
        let problem = body_string(resp).await;
        assert!(
            !problem.contains(SECRET_TOKEN),
            "capability-gate body must never carry a secret: {problem}"
        );
    }

    // `resolve` reaches the handler (the tenancy middleware special-cases
    // the literal `resolve` sub-route so it is NOT parsed as a `{cred}`
    // CredentialId — regression guard for the route-shadow) and a static
    // type completes synchronously: 200 + persisted id, no secret echo.
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials/resolve"),
            &token,
            &serde_json::json!({
                "credential_key": "api_key",
                "data": { "api_key": SECRET_TOKEN }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "resolve must reach the handler and complete for a static type — \
         NOT a pre-handler 404 (the tenancy route-shadow this guards against)"
    );
    let resolved = body_string(resp).await;
    assert!(
        !resolved.contains(SECRET_TOKEN),
        "resolve response must not echo the submitted secret: {resolved}"
    );
    let resolved_json: serde_json::Value = serde_json::from_str(&resolved).unwrap();
    assert_eq!(resolved_json["status"], "complete");
    assert!(
        resolved_json["credential_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("cred_")),
        "complete resolve must return the persisted credential id"
    );

    // `resolve/continue` with a well-formed typed payload on a
    // non-interactive type → 400 from the capability gate; the submitted
    // material never echoes.
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials/resolve/continue"),
            &token,
            &serde_json::json!({
                "credential_key": "api_key",
                "pending_token": "tok",
                "user_input": { "Code": { "code": SECRET_TOKEN } }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "continue on a non-interactive type must fail the capability gate \
         with 400 — and must reach the handler, not 404 in tenancy"
    );
    let problem = body_string(resp).await;
    assert!(
        !problem.contains(SECRET_TOKEN),
        "continue error body must not echo caller-submitted credential \
         material (SECRET_TOKEN was in `user_input`): {problem}"
    );

    // Type-discovery endpoints (system-level, not workspace-scoped) are
    // registry-backed: the first-party catalog lists the builtin types and
    // a known key resolves.
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_get("/api/v1/credentials/types", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
    let keys: Vec<&str> = listed["types"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["key"].as_str())
        .collect();
    assert!(
        keys.contains(&"api_key"),
        "registry-backed catalog must list api_key, got {keys:?}"
    );

    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_get("/api/v1/credentials/types/api_key", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "known type key resolves");

    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_get("/api/v1/credentials/types/no_such_type", &token))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "unknown credential type ⇒ 404 (types are public catalog info; \
         non-existence disclosure is non-sensitive — unlike credential instances)"
    );
}
/// Guard-precision regression (Copilot review, PR #674): the
/// `"credentials" if segments[7] != "resolve"` tenancy match guard must
/// keep rejecting a **non-`resolve`, non-ULID** `{cred}` segment
/// *before* the handler. Locks the other half of the contract so a
/// future broadening of the guard cannot silently weaken `{cred}` ULID
/// validation — the route-shadow fix only carves out the literal
/// `resolve`, nothing else.
#[tokio::test]
async fn tenancy_still_rejects_non_ulid_cred_segment_before_handler() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    // "not-a-valid-ulid" is neither the literal `resolve` sub-route nor a
    // `cred_<ULID>` → the tenancy `{cred}` matcher must 404 before any
    // handler runs (bare `{cred}` GET, and the `{cred}/test` POST
    // sub-route — both put the bad segment at the `{cred}` position).
    let bad = "not-a-valid-ulid";

    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_get(&ws_path(&format!("/credentials/{bad}")), &token))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "GET /credentials/{bad}: a non-`resolve`, non-ULID `{{cred}}` segment \
         must be rejected by tenancy ULID validation *before* the handler — \
         the route-shadow guard must not weaken `{{cred}}` validation"
    );

    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json(
            "POST",
            &ws_path(&format!("/credentials/{bad}/test")),
            &token,
            &serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "POST /credentials/{bad}/test: the `{{cred}}` segment is still strictly \
         ULID-validated by tenancy before the handler"
    );
}
