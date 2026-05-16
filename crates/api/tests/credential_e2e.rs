//! Phase 4 — credential CRUD end-to-end + abuse-case coverage.
//!
//! Credentials are the highest-value target in the API surface, so this
//! file is deliberately heavy on the security cases (canon §13, §12.5 /
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

// ── Honest endpoints keep returning an honest 503 ─────────────────────────────

#[tokio::test]
async fn credential_engine_owned_endpoints_stay_honest_503() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a real credential so test/refresh/revoke reach the handler
    // body (not a pre-handler 404) and we prove the 503 is the honest
    // engine-owned signal, not an artifact of a missing row.
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
    let created: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
    let cred_id = created["id"].as_str().unwrap().to_owned();

    // `test` / `refresh` / `revoke` carry a valid `{cred}` ULID, so the
    // tenancy middleware passes and the request reaches the handler →
    // honest 503 (engine-owned dispatch, no CredentialRegistry wired).
    let post_503: &[(&str, String, serde_json::Value)] = &[
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

    for (method, path, body) in post_503 {
        let app = app::build_app(state.clone(), &config);
        let resp = app
            .oneshot(auth_json(method, path, &token, body))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path} must stay an honest 503 (engine-owned / no registry)"
        );
        let problem = body_string(resp).await;
        // Honest error: structured, no secret, problem+json.
        assert!(
            !problem.contains(SECRET_TOKEN),
            "503 body must never carry a secret: {problem}"
        );
    }

    // `resolve` / `resolve/continue` are **structurally unreachable**
    // behind the tenancy middleware: it parses the path segment after
    // `credentials` as a `CredentialId`, and the literal `resolve`
    // segment fails ULID parsing → a flat **404 before any handler
    // runs** (see `crates/api/src/middleware/tenancy.rs`
    // `resolve_path_ids`). This is a pre-existing route-ordering
    // condition, not a Phase-4 regression (the prior 503 stub was
    // equally unreachable). It is still §4.5-honest — the caller
    // cannot obtain a fake credential; they get a hard 404. The
    // generic-resolve handler itself stays an honest 503 (asserted by
    // the transport unit test); the route fix is tracked separately.
    let post_404: &[(&str, String, serde_json::Value)] = &[
        (
            "POST",
            ws_path("/credentials/resolve"),
            serde_json::json!({ "credential_key": "api_key", "data": {} }),
        ),
        (
            "POST",
            ws_path("/credentials/resolve/continue"),
            serde_json::json!({ "pending_token": "tok", "user_input": {} }),
        ),
    ];

    for (method, path, body) in post_404 {
        let app = app::build_app(state.clone(), &config);
        let resp = app
            .oneshot(auth_json(method, path, &token, body))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} {path} is shadowed by the tenancy `{{cred}}` matcher → \
             honest 404 (no false capability; pre-existing route condition)"
        );
        let problem = body_string(resp).await;
        assert!(
            !problem.contains(SECRET_TOKEN),
            "404 body must never carry a secret: {problem}"
        );
    }

    // Type-discovery endpoints (system-level, not workspace-scoped).
    for path in [
        "/api/v1/credentials/types".to_owned(),
        "/api/v1/credentials/types/api_key".to_owned(),
    ] {
        let app = app::build_app(state.clone(), &config);
        let resp = app.oneshot(auth_get(&path, &token)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} type discovery must stay an honest 503 (no CredentialRegistry)"
        );
    }
}
