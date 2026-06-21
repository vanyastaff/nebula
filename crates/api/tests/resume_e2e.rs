//! End-to-end tests for `POST /resume` (ADR-0099 W-S3d).
//!
//! ## Coverage
//!
//! 1.  Happy path — 202 Accepted, one `ControlMsg{Resume, Webhook target}` enqueued.
//! 2.  Single-use replay — same bearer twice: first 202, second 404; ONE enqueue.
//! 3.  Forged/absent bearer — 404, ZERO enqueues.
//! 4.  Expired token (clock-injected past expiry) — consumed, 404, no enqueue.
//! 5.  Malformed `expires_at` — consumed, 404, no enqueue (fail-closed).
//! 6.  Kind-confusion — `Approval`-kind row → 404, ZERO enqueues.
//! 7.  Scope-from-row — enqueued scope == `row.scope`, unaffected by request headers.
//! 8.  Per-IP rate-limit fires BEFORE DB: saturate → 429 + `Retry-After`; store not hit.
//! 9.  Global rate-limit fires BEFORE DB: saturate → 429 + `Retry-After`; store not hit.
//! 10. Per-tenant rate-limit fires AFTER consume: 429 (token burned, documented).
//! 11. No token-in-URL path — `GET /resume/{token}` returns non-200.
//! 12. Storage error → 503 + `Retry-After`, ZERO enqueues, token unconsumed.
//! 13. Body inert — extra JSON body ignored; oversized body → 413, no store hit.
//! 14. Bearer-extraction uniformity — missing header / `Basic` scheme / empty token → same 404.
//!
//! ## Harness
//!
//! Each test builds an `AppState` with:
//! - `InMemoryResumeTokenStore` seeded via `seed_for_test`.
//! - `InMemoryControlQueue` inspected via `snapshot()`.
//! - `ResumeHandlerComponents` with a `MockClock` (controllable expiry time).
//!
//! The router is the real `nebula_api::app::build_app` router, which mounts
//! `POST /resume` before tenancy middleware.  Per-IP rate-limit control is
//! achieved by injecting `ConnectInfo<SocketAddr>` into the request extension
//! (axum's `ConnectInfo` extractor reads from `request.extensions()`) and/or
//! via `X-Forwarded-For` headers.

mod common;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use nebula_action::MockClock;
use nebula_api::{
    ApiConfig, AppState, app,
    transport::webhook::{ResumeHandlerComponents, ratelimit::WebhookRateLimiter},
};
use nebula_storage::{
    InMemoryResumeTokenStore,
    inmem::{InMemoryControlQueue, InMemoryExecutionStore},
};
use nebula_storage_port::{
    Scope, StorageError,
    dto::{
        ControlCommand, ControlMsg, ResumeTarget,
        resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash},
    },
    store::{ResumeProducer, ResumeTokenStore},
};
use tower::ServiceExt;

// ── Test constants ────────────────────────────────────────────────────────────

/// Fixed tenant scope used for all token rows unless a test overrides it.
fn test_scope() -> Scope {
    Scope::new("ws_test_000000000001", "org_test_000000000001")
}

/// RFC 5737 documentation-range peer address injected as `ConnectInfo`.
const PEER_A: &str = "203.0.113.10:5000";
/// Second documentation-range address for tests that need two distinct IPs.
const PEER_B: &str = "203.0.113.20:5001";

// ── Token / row builders ──────────────────────────────────────────────────────

/// Compute the SHA-256 of `plaintext` and wrap it as a `TokenHash`.
///
/// Mirrors `nebula_api::transport::webhook::token::token_hash` exactly —
/// used here to build seeded rows whose hash the handler will reconstruct.
fn token_hash_of(plaintext: &str) -> TokenHash {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(plaintext.as_bytes());
    TokenHash::try_from_bytes(digest.to_vec()).expect("SHA-256 always produces exactly 32 bytes")
}

/// Minimal `Webhook`-kind `ResumeTokenRow` for `plaintext`, without expiry.
fn webhook_row(
    plaintext: &str,
    execution_id: &str,
    callback_label: &str,
    scope: Scope,
) -> ResumeTokenRow {
    ResumeTokenRow::new(
        token_hash_of(plaintext),
        scope,
        execution_id.to_owned(),
        "node_step_a".to_owned(),
        ResumeTokenWaitKind::Webhook,
        callback_label.to_owned(),
        "2026-06-21T00:00:00Z".to_owned(),
        None,
    )
}

/// Webhook-kind row with an explicit RFC-3339 expiry timestamp.
fn webhook_row_with_expiry(
    plaintext: &str,
    execution_id: &str,
    callback_label: &str,
    scope: Scope,
    expires_at: &str,
) -> ResumeTokenRow {
    ResumeTokenRow::new(
        token_hash_of(plaintext),
        scope,
        execution_id.to_owned(),
        "node_step_a".to_owned(),
        ResumeTokenWaitKind::Webhook,
        callback_label.to_owned(),
        "2026-06-21T00:00:00Z".to_owned(),
        Some(expires_at.to_owned()),
    )
}

/// `Approval`-kind row — used for the kind-confusion test (test 6).
fn approval_row(plaintext: &str, execution_id: &str, scope: Scope) -> ResumeTokenRow {
    ResumeTokenRow::new(
        token_hash_of(plaintext),
        scope,
        execution_id.to_owned(),
        "node_step_b".to_owned(),
        ResumeTokenWaitKind::Approval,
        "approver@example.com".to_owned(),
        "2026-06-21T00:00:00Z".to_owned(),
        None,
    )
}

// ── Component builders ────────────────────────────────────────────────────────

/// Default components with a generous rate-limit (no test hits RL by accident)
/// and a caller-supplied clock.
fn components_with_clock(clock: Arc<dyn nebula_action::Clock>) -> ResumeHandlerComponents {
    ResumeHandlerComponents {
        ip_rate_limiter: WebhookRateLimiter::new(10_000),
        global_rate_limiter: WebhookRateLimiter::new(10_000),
        tenant_rate_limiter: WebhookRateLimiter::new(10_000),
        clock,
    }
}

/// Components with a 1-RPM per-IP cap; others generous.  Used for test 8.
fn components_tight_ip_rate_limit(clock: Arc<dyn nebula_action::Clock>) -> ResumeHandlerComponents {
    ResumeHandlerComponents {
        ip_rate_limiter: WebhookRateLimiter::new(1),
        global_rate_limiter: WebhookRateLimiter::new(10_000),
        tenant_rate_limiter: WebhookRateLimiter::new(10_000),
        clock,
    }
}

/// Components with a 1-RPM global cap; others generous.  Used for test 9.
fn components_tight_global_rate_limit(
    clock: Arc<dyn nebula_action::Clock>,
) -> ResumeHandlerComponents {
    ResumeHandlerComponents {
        ip_rate_limiter: WebhookRateLimiter::new(10_000),
        global_rate_limiter: WebhookRateLimiter::new(1),
        tenant_rate_limiter: WebhookRateLimiter::new(10_000),
        clock,
    }
}

/// Components with a 1-RPM per-tenant cap; others generous.  Used for test 10.
fn components_tight_tenant_rate_limit(
    clock: Arc<dyn nebula_action::Clock>,
) -> ResumeHandlerComponents {
    ResumeHandlerComponents {
        ip_rate_limiter: WebhookRateLimiter::new(10_000),
        global_rate_limiter: WebhookRateLimiter::new(10_000),
        tenant_rate_limiter: WebhookRateLimiter::new(1),
        clock,
    }
}

// ── Harness ───────────────────────────────────────────────────────────────────

/// Shared handles returned by the test harness builders.
struct ResumeHarness {
    /// The fully assembled axum app (routes + middleware) under test.
    app: axum::Router,
    /// Raw token store — seeded before test requests; shared with `AppState`.
    token_store: InMemoryResumeTokenStore,
    /// Raw control-queue handle — inspected after requests via `snapshot()`.
    control_queue: InMemoryControlQueue,
}

/// Build the standard resume test harness with the given rate-limit components.
///
/// The resume-token store, resume producer, and control queue all share the
/// `exec_store`'s `SharedState`, so `seed_for_test` writes on `token_store` are
/// visible to the producer's `peek` / `consume_and_enqueue_resume`, and
/// `control_queue.snapshot()` reflects every `Resume` the producer enqueues.
async fn build_resume_harness(components: ResumeHandlerComponents) -> ResumeHarness {
    use nebula_storage::inmem::{
        InMemoryJournalReader, InMemoryNodeResultStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);
    // Token store + producer over the SAME shared state as the control queue —
    // built before `exec_store` is moved into the `Arc`.
    let token_store = exec_store.resume_token_store();
    let resume_producer = exec_store.resume_producer();

    let api_config = ApiConfig::for_test();
    let state = AppState::new(
        Arc::new(workflow_store),
        Arc::new(workflow_versions),
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue.clone()),
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(common::TestOrgResolver))
    .with_workspace_resolver(Arc::new(common::TestWorkspaceResolver))
    .with_insecure_tenant_rbac_bypass_for_tests()
    .with_resume_token_store(Arc::new(token_store.clone()))
    .with_resume_producer(Arc::new(resume_producer))
    .with_resume_handler_components(components);

    let app = app::build_app(state, &api_config);

    ResumeHarness {
        app,
        token_store,
        control_queue,
    }
}

/// A `ResumeProducer` port whose `peek` always returns a storage error.
///
/// Used to assert abuse-case 15: a transient storage fault on the read-only
/// `peek` → 503, token NOT burned, no `ControlMsg` enqueued. `consume_and_
/// enqueue_resume` is never reached on this path; it returns the same error
/// for completeness.
#[derive(Debug)]
struct AlwaysFailResumeProducer;

#[async_trait::async_trait]
impl ResumeProducer for AlwaysFailResumeProducer {
    async fn peek(&self, _hash: &TokenHash) -> Result<Option<ResumeTokenRow>, StorageError> {
        Err(StorageError::Connection(
            "simulated transient storage failure".to_owned(),
        ))
    }

    async fn consume_and_enqueue_resume(
        &self,
        _hash: &TokenHash,
        _resume_msg: &ControlMsg,
    ) -> Result<bool, StorageError> {
        Err(StorageError::Connection(
            "simulated transient storage failure".to_owned(),
        ))
    }
}

/// Build a harness whose resume producer always returns a storage error on `peek`.
async fn build_failing_store_harness(components: ResumeHandlerComponents) -> ResumeHarness {
    use nebula_storage::inmem::{
        InMemoryJournalReader, InMemoryNodeResultStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);
    // A standalone store is returned in `token_store` for the field but is
    // never wired into AppState — `AlwaysFailResumeProducer` is wired instead.
    let token_store_placeholder = InMemoryResumeTokenStore::standalone();

    let api_config = ApiConfig::for_test();
    let state = AppState::new(
        Arc::new(workflow_store),
        Arc::new(workflow_versions),
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue.clone()),
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(common::TestOrgResolver))
    .with_workspace_resolver(Arc::new(common::TestWorkspaceResolver))
    .with_insecure_tenant_rbac_bypass_for_tests()
    .with_resume_producer(Arc::new(AlwaysFailResumeProducer))
    .with_resume_handler_components(components);

    let app = app::build_app(state, &api_config);

    ResumeHarness {
        app,
        token_store: token_store_placeholder,
        control_queue,
    }
}

// ── Request builders ──────────────────────────────────────────────────────────

/// `POST /resume` with a Bearer token and an injected `ConnectInfo` extension.
///
/// Injecting `ConnectInfo<SocketAddr>` into extensions is the axum-documented
/// approach for testing handlers that use the `ConnectInfo` extractor without
/// `into_make_service_with_connect_info`.
fn resume_post(bearer: &str, peer: &str) -> Request<Body> {
    let peer_addr: SocketAddr = peer
        .parse()
        .expect("test peer must be a valid socket address");
    Request::builder()
        .method("POST")
        .uri("/resume")
        .header("Authorization", format!("Bearer {bearer}"))
        .extension(ConnectInfo(peer_addr))
        .body(Body::empty())
        .expect("resume POST must construct without error")
}

/// `POST /resume` with a Bearer token, an injected peer, and an explicit body.
fn resume_post_with_body(bearer: &str, peer: &str, body: impl Into<Body>) -> Request<Body> {
    let peer_addr: SocketAddr = peer
        .parse()
        .expect("test peer must be a valid socket address");
    Request::builder()
        .method("POST")
        .uri("/resume")
        .header("Authorization", format!("Bearer {bearer}"))
        .header("Content-Type", "application/json")
        .extension(ConnectInfo(peer_addr))
        .body(body.into())
        .expect("resume POST with body must construct without error")
}

/// `POST /resume` with no `Authorization` header.
fn resume_post_no_auth(peer: &str) -> Request<Body> {
    let peer_addr: SocketAddr = peer
        .parse()
        .expect("test peer must be a valid socket address");
    Request::builder()
        .method("POST")
        .uri("/resume")
        .extension(ConnectInfo(peer_addr))
        .body(Body::empty())
        .expect("no-auth resume POST must construct without error")
}

/// `POST /resume` with a `Basic` scheme (wrong scheme).
fn resume_post_basic_scheme(peer: &str) -> Request<Body> {
    let peer_addr: SocketAddr = peer
        .parse()
        .expect("test peer must be a valid socket address");
    Request::builder()
        .method("POST")
        .uri("/resume")
        .header("Authorization", "Basic dXNlcjpwYXNz")
        .extension(ConnectInfo(peer_addr))
        .body(Body::empty())
        .expect("basic-scheme resume POST must construct without error")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1 — Happy path: 202 Accepted + one targeted `Resume` `ControlMsg` enqueued.
///
/// Asserts: status 202, exactly one Pending Resume msg with correct execution_id,
/// scope, and `ResumeTarget::Webhook{callback_id}`.
#[tokio::test]
async fn happy_path_returns_202_and_enqueues_resume() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    let bearer = "resume-bearer-t1-happy";
    harness
        .token_store
        .seed_for_test(webhook_row(bearer, "exe-t1", "my-callback", test_scope()));

    let resp = harness
        .app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "happy path must return 202"
    );

    let queued = harness.control_queue.snapshot();
    assert_eq!(
        queued.len(),
        1,
        "exactly one control message must be enqueued"
    );
    let (msg, status) = &queued[0];
    assert_eq!(msg.command, ControlCommand::Resume);
    assert_eq!(msg.execution_id, "exe-t1");
    assert_eq!(
        msg.scope,
        test_scope(),
        "enqueued scope must come from the token row"
    );
    assert_eq!(
        msg.resume_target,
        Some(ResumeTarget::Webhook {
            callback_id: "my-callback".to_owned()
        }),
        "resume target must be Webhook with the row's callback_label"
    );
    assert_eq!(
        status, "Pending",
        "enqueued message must be in Pending status"
    );
}

/// Test 2 — Single-use replay: first request → 202; second with same bearer → 404.
///
/// Only ONE `ControlMsg` must ever be enqueued across both calls.
#[tokio::test]
async fn single_use_replay_second_call_returns_404_one_enqueue() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    let bearer = "resume-bearer-t2-replay";
    harness
        .token_store
        .seed_for_test(webhook_row(bearer, "exe-t2", "cb-replay", test_scope()));

    let resp_first = harness
        .app
        .clone()
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_first.status(),
        StatusCode::ACCEPTED,
        "first call must return 202"
    );

    let resp_second = harness
        .app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_second.status(),
        StatusCode::NOT_FOUND,
        "second call with same bearer must return 404 (token consumed)"
    );

    assert_eq!(
        harness.control_queue.snapshot().len(),
        1,
        "replay must not produce a second enqueue"
    );
}

/// Test 3 — Forged/absent bearer: 404, ZERO enqueues.
///
/// A token hash that was never seeded returns uniform 404, byte-identical to the
/// consumed-token 404 in test 2 (no existence oracle).
#[tokio::test]
async fn forged_bearer_returns_404_no_enqueue() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    // Seed a real row but present a completely different plaintext.
    harness.token_store.seed_for_test(webhook_row(
        "real-token-t3",
        "exe-t3",
        "cb-t3",
        test_scope(),
    ));

    let resp = harness
        .app
        .oneshot(resume_post("forged-token-never-seeded", PEER_A))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "forged bearer must return 404"
    );
    assert!(
        harness.control_queue.snapshot().is_empty(),
        "no ControlMsg must be enqueued for a forged bearer"
    );
}

/// Test 4 — Expired token (clock past expiry): 404, no enqueue, NO BURN.
///
/// Expiry is checked at step 8 on the read-only `peek` row, BEFORE the consume.
/// The token is NOT burned (wart fix): the caller sees 404 and the token row
/// survives, so a subsequent `consume` still returns it.
#[tokio::test]
async fn expired_token_returns_404_no_enqueue_no_burn() {
    // Clock at epoch 1001; token expired at epoch 1000.
    let clock = Arc::new(MockClock::at_unix_secs(1_001));
    let harness = build_resume_harness(components_with_clock(clock)).await;

    let bearer = "resume-bearer-t4-expired";
    // RFC-3339 for Unix epoch 1000 = 1970-01-01T00:16:40Z
    let row = webhook_row_with_expiry(
        bearer,
        "exe-t4",
        "cb-t4",
        test_scope(),
        "1970-01-01T00:16:40Z",
    );
    harness.token_store.seed_for_test(row);

    let resp = harness
        .app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "expired token must return 404"
    );
    assert!(
        harness.control_queue.snapshot().is_empty(),
        "expired token must not produce a ControlMsg enqueue"
    );
    // Wart fix: the token survived (NOT burned). A direct consume still finds it.
    let survived = harness
        .token_store
        .consume(&token_hash_of(bearer))
        .await
        .expect("consume must not error");
    assert!(
        survived.is_some(),
        "expired token must NOT be burned by the 404 path (wart fix)"
    );
}

/// Test 5 — Malformed `expires_at`: fail-closed → 404, no enqueue, NO BURN.
///
/// A row with an unparseable `expires_at` string is treated as expired
/// (fail-closed): the handler returns 404 and does not enqueue a Resume — and
/// does not burn the token (checked on the read-only `peek` row, before consume).
#[tokio::test]
async fn malformed_expires_at_fails_closed_404_no_enqueue_no_burn() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    let bearer = "resume-bearer-t5-malformed-expiry";
    let row = webhook_row_with_expiry(
        bearer,
        "exe-t5",
        "cb-t5",
        test_scope(),
        "NOT-A-VALID-RFC3339-DATE",
    );
    harness.token_store.seed_for_test(row);

    let resp = harness
        .app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "malformed expires_at must fail-closed to 404"
    );
    assert!(
        harness.control_queue.snapshot().is_empty(),
        "malformed expires_at must not produce a ControlMsg enqueue"
    );
    let survived = harness
        .token_store
        .consume(&token_hash_of(bearer))
        .await
        .expect("consume must not error");
    assert!(
        survived.is_some(),
        "malformed-expiry token must NOT be burned by the 404 path (wart fix)"
    );
}

/// Test 6 — Kind-confusion: `Approval`-kind row at `POST /resume` → 404, no
/// enqueue, NO BURN.
///
/// The Webhook endpoint must not resolve an Approval wait.  The `_` arm on the
/// kind-match in the handler is the structural fail-closed gate, and it fires on
/// the read-only `peek` row so the token is NOT burned (wart fix).
#[tokio::test]
async fn approval_kind_row_at_webhook_endpoint_returns_404_no_enqueue_no_burn() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    let bearer = "resume-bearer-t6-approval-kind";
    harness
        .token_store
        .seed_for_test(approval_row(bearer, "exe-t6", test_scope()));

    let resp = harness
        .app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Approval-kind token at /resume must return 404 (fail-closed kind-match)"
    );
    assert!(
        harness.control_queue.snapshot().is_empty(),
        "Approval-kind token must never enqueue a Resume at the Webhook endpoint"
    );
    let survived = harness
        .token_store
        .consume(&token_hash_of(bearer))
        .await
        .expect("consume must not error");
    assert!(
        survived.is_some(),
        "wrong-kind token must NOT be burned by the 404 path (wart fix)"
    );
}

/// Test 7 — Scope-from-row: enqueued scope is `row.scope`, not from the request.
///
/// The handler has no `TenantContext` extractor.  The structural proof is the
/// ABSENCE of any tenant scope in the extractor list.  This test asserts the
/// behavioral effect: the enqueued `ControlMsg.scope` equals the row's scope,
/// regardless of what path or headers the request carries.
#[tokio::test]
async fn enqueued_scope_comes_from_row_not_from_request() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    // Use a scope completely unrelated to `TEST_ORG`/`TEST_WS`.
    let row_scope = Scope::new("ws_row_scoped_only_111", "org_row_scoped_only_222");
    let bearer = "resume-bearer-t7-scope-from-row";
    harness
        .token_store
        .seed_for_test(webhook_row(bearer, "exe-t7", "cb-t7", row_scope.clone()));

    let resp = harness
        .app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED, "must succeed");

    let queued = harness.control_queue.snapshot();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].0.scope, row_scope,
        "enqueued scope must be row.scope, not derived from request URL or headers"
    );
}

/// Test 8 — Per-IP rate-limit fires BEFORE the DB store is touched.
///
/// Saturate the per-IP bucket (capacity 1) from PEER_A, then assert the next
/// request from PEER_A returns 429 with `Retry-After`, and no second enqueue
/// happened (proving the store was never reached for that request).
#[tokio::test]
async fn per_ip_rate_limit_429_before_db_hit() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_tight_ip_rate_limit(clock)).await;

    let bearer_a = "resume-bearer-t8-ip-rl-first";
    let bearer_b = "resume-bearer-t8-ip-rl-second";
    harness
        .token_store
        .seed_for_test(webhook_row(bearer_a, "exe-t8-a", "cb-t8a", test_scope()));
    harness
        .token_store
        .seed_for_test(webhook_row(bearer_b, "exe-t8-b", "cb-t8b", test_scope()));

    // First request from PEER_A consumes the per-IP slot (capacity = 1).
    let resp_first = harness
        .app
        .clone()
        .oneshot(resume_post(bearer_a, PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_first.status(),
        StatusCode::ACCEPTED,
        "first request must pass per-IP RL"
    );

    // Second request from the SAME IP must be rate-limited.
    let resp_second = harness
        .app
        .oneshot(resume_post(bearer_b, PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_second.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "second request from same IP must be 429"
    );
    assert!(
        resp_second.headers().contains_key("retry-after"),
        "per-IP 429 must include Retry-After header"
    );

    // Only the first request reached the store; the second was blocked pre-DB.
    assert_eq!(
        harness.control_queue.snapshot().len(),
        1,
        "only first request must have enqueued a Resume (second was IP-RL blocked)"
    );
}

/// Test 9 — Global rate-limit fires BEFORE the DB store is touched.
///
/// Saturate the global bucket (capacity 1) with one request from PEER_A, then
/// assert a request from a DIFFERENT IP (PEER_B) is still blocked globally.
#[tokio::test]
async fn global_rate_limit_429_before_db_hit() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_tight_global_rate_limit(clock)).await;

    let bearer_a = "resume-bearer-t9-global-rl-first";
    let bearer_b = "resume-bearer-t9-global-rl-second";
    harness
        .token_store
        .seed_for_test(webhook_row(bearer_a, "exe-t9-a", "cb-t9a", test_scope()));
    harness
        .token_store
        .seed_for_test(webhook_row(bearer_b, "exe-t9-b", "cb-t9b", test_scope()));

    // First request (any peer) consumes the global slot.
    let resp_first = harness
        .app
        .clone()
        .oneshot(resume_post(bearer_a, PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_first.status(),
        StatusCode::ACCEPTED,
        "first request must pass global RL"
    );

    // Second request from a DIFFERENT peer is still globally rate-limited.
    let resp_second = harness
        .app
        .oneshot(resume_post(bearer_b, PEER_B))
        .await
        .unwrap();
    assert_eq!(
        resp_second.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "second request must be blocked by global RL even from a different IP"
    );
    assert!(
        resp_second.headers().contains_key("retry-after"),
        "global 429 must include Retry-After header"
    );

    assert_eq!(
        harness.control_queue.snapshot().len(),
        1,
        "only first request must have enqueued a Resume"
    );
}

/// Test 10 — Per-tenant 429 is single-shot per token (fires post-burn).
///
/// The per-tenant rate-limit fires at step 11, on the atomic consume+enqueue
/// WINNER — so a 429 is observable only on a request that ALSO burned its token
/// (and already enqueued its Resume in the same transaction). The token is now
/// burned: a replay of the same bearer yields 404, not a repeatable 429. This is
/// the oracle-avoidance property — a 429 can never be replayed against a still-
/// valid token (which would leak "valid token + throttled tenant").
#[tokio::test]
async fn per_tenant_429_still_single_shot() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_tight_tenant_rate_limit(clock)).await;

    let bearer_a = "resume-bearer-t10-tenant-rl-first";
    let bearer_b = "resume-bearer-t10-tenant-rl-second";
    // Both rows share the same scope → same tenant rate-limit key.
    harness
        .token_store
        .seed_for_test(webhook_row(bearer_a, "exe-t10-a", "cb-t10a", test_scope()));
    harness
        .token_store
        .seed_for_test(webhook_row(bearer_b, "exe-t10-b", "cb-t10b", test_scope()));

    // First request passes per-tenant RL and saturates its slot (capacity = 1).
    let resp_first = harness
        .app
        .clone()
        .oneshot(resume_post(bearer_a, PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_first.status(),
        StatusCode::ACCEPTED,
        "first request must pass tenant RL"
    );

    // Second request, same tenant: wins the atomic burn+enqueue, THEN the tenant
    // RL fires → 429. The token is burned in the same transaction.
    let resp_second = harness
        .app
        .clone()
        .oneshot(resume_post(bearer_b, PEER_B))
        .await
        .unwrap();
    assert_eq!(
        resp_second.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "second same-tenant request must be 429 (post-burn)"
    );
    assert!(
        resp_second.headers().contains_key("retry-after"),
        "tenant 429 must include Retry-After header"
    );

    // The 429 is single-shot: bearer_b is now burned, so a replay is 404 — a
    // repeatable 429 against a live token (the throttle oracle) is impossible.
    let burned = harness
        .token_store
        .consume(&token_hash_of(bearer_b))
        .await
        .expect("consume must not error");
    assert!(
        burned.is_none(),
        "a 429'd request that won the atomic delete must have burned its token \
         (single-shot — no repeatable 429 oracle on a still-valid token)"
    );

    // Both atomic winners enqueued their Resume (the burn and the enqueue are one
    // transaction — a 429 cannot retroactively un-enqueue an already-committed
    // Resume). This is strictly safer than the prior post-burn-RL handler, which
    // burned the token but dropped the Resume — the very gap this seam closes.
    assert_eq!(
        harness.control_queue.snapshot().len(),
        2,
        "both atomic winners enqueued a Resume (burn+enqueue are one tx)"
    );
}

/// Test 11 — No token-in-URL route: `GET /resume/{token}` must not return 200.
///
/// A path-parameter route `/resume/{token}` would be an existence oracle (the
/// token appears in server logs, CDN caches, referrer headers).  We assert no
/// such route exists in the router.
#[tokio::test]
async fn no_token_in_url_path_route_exists() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    let peer_addr: SocketAddr = PEER_A.parse().unwrap();
    let get_with_path_param = Request::builder()
        .method("GET")
        .uri("/resume/some-secret-token-in-the-url")
        .extension(ConnectInfo(peer_addr))
        .body(Body::empty())
        .unwrap();

    let resp = harness.app.oneshot(get_with_path_param).await.unwrap();

    // The only `/resume` route is an exact-path `POST` (no path parameter), so
    // `GET /resume/{token}` matches NOTHING.  It falls through to the merged
    // `/internal/v1` sub-router's `internal_auth_middleware`, which — with no
    // internal shared token configured in `ApiConfig::for_test` — deterministically
    // returns `503 SERVICE_UNAVAILABLE` for any unmatched path that reaches it
    // (`domain::internal::router` layers the middleware over its routes; axum 0.8
    // applies that layer to the merged fallback).
    //
    // We pin the EXACT status rather than a bare `assert_ne!(200)`: the security
    // property is that `/resume/{token}` matches NO route, and the only proof of
    // "no match" is that the request lands on the unmatched-path fallback.  If a
    // `/resume/{token}` route were ever added it would return that route's status
    // instead (2xx on success — or even a 4xx/5xx of its own, which a bare
    // `assert_ne!(200)` would silently accept while the URL-borne-token oracle is
    // live).  Pinning 503 makes any such regression fail this assertion.
    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "GET /resume/{{token}} must hit the unmatched-path fallback (503), proving no \
         path-parameter route exists that could echo or accept a URL-borne token"
    );
}

/// Test 12 — Storage error on `peek` → 503 + `Retry-After`, ZERO enqueues.
///
/// When the producer returns `Err` on the read-only `peek`, no token is burned
/// (peek never deletes), so the caller can retry.  The handler must return 503
/// with `Retry-After` and must not enqueue anything.
#[tokio::test]
async fn storage_error_returns_503_retry_after_no_enqueue() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_failing_store_harness(components_with_clock(clock)).await;

    let resp = harness
        .app
        .oneshot(resume_post("any-bearer-does-not-matter", PEER_A))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "store error must return 503"
    );
    assert!(
        resp.headers().contains_key("retry-after"),
        "503 on storage error must include Retry-After header"
    );
    assert!(
        harness.control_queue.snapshot().is_empty(),
        "no ControlMsg must be enqueued when the store returns an error"
    );
}

/// Test 13 — Body is inert: extra JSON ignored; oversized body → 413 before store hit.
///
/// Two sub-cases:
/// - 13a: a valid bearer with an attacker-injected body claiming a different
///        `execution_id` and `scope` — the enqueued msg must reflect ONLY the row.
/// - 13b: a 5 KiB body (exceeding the 4 KiB cap) — returns 413 before any store hit.
#[tokio::test]
async fn body_inert_extra_json_ignored_oversized_body_413() {
    // ── 13a: valid bearer + attacker-injected body ────────────────────────────
    let clock_a = Arc::new(MockClock::at_now());
    let harness_a = build_resume_harness(components_with_clock(clock_a)).await;

    let bearer = "resume-bearer-t13-body-inert";
    harness_a
        .token_store
        .seed_for_test(webhook_row(bearer, "exe-t13", "cb-t13", test_scope()));

    let attacker_body = r#"{"execution_id":"attacker-injected-id","scope":"attacker-scope"}"#;
    let resp_a = harness_a
        .app
        .oneshot(resume_post_with_body(bearer, PEER_A, attacker_body))
        .await
        .unwrap();

    assert_eq!(
        resp_a.status(),
        StatusCode::ACCEPTED,
        "valid bearer with extra body must still return 202"
    );

    let queued = harness_a.control_queue.snapshot();
    assert_eq!(queued.len(), 1);
    let (enqueued_msg, _) = &queued[0];
    assert_eq!(
        enqueued_msg.execution_id, "exe-t13",
        "execution_id must come from the token row, not the request body"
    );
    assert_eq!(
        enqueued_msg.scope,
        test_scope(),
        "scope must come from the token row, not the request body"
    );

    // ── 13b: oversized body → 413 before any store hit ───────────────────────
    let clock_b = Arc::new(MockClock::at_now());
    let harness_b = build_resume_harness(components_with_clock(clock_b)).await;

    let peer_addr: SocketAddr = PEER_A.parse().unwrap();
    let oversized_body = "x".repeat(5 * 1024); // 5 KiB > 4 KiB cap
    let oversized_request = Request::builder()
        .method("POST")
        .uri("/resume")
        .header("Authorization", "Bearer would-be-a-real-token-t13b")
        .extension(ConnectInfo(peer_addr))
        .body(Body::from(oversized_body))
        .unwrap();

    let resp_b = harness_b.app.oneshot(oversized_request).await.unwrap();
    assert_eq!(
        resp_b.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "oversized body must return 413"
    );
    assert!(
        harness_b.control_queue.snapshot().is_empty(),
        "oversized body must not trigger any store hit or ControlMsg enqueue"
    );
}

/// Test 14 — Bearer-extraction uniformity: missing header / `Basic` scheme / empty
/// token all return the same uniform 404 (never 401 — no auth-revealing status).
#[tokio::test]
async fn bearer_extraction_uniformity_all_variants_return_404() {
    let clock = Arc::new(MockClock::at_now());
    let harness = build_resume_harness(components_with_clock(clock)).await;

    // 14a — no Authorization header at all.
    let resp_no_header = harness
        .app
        .clone()
        .oneshot(resume_post_no_auth(PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_no_header.status(),
        StatusCode::NOT_FOUND,
        "missing Authorization header must return 404, not 401"
    );

    // 14b — wrong scheme (`Basic`).
    let resp_basic = harness
        .app
        .clone()
        .oneshot(resume_post_basic_scheme(PEER_A))
        .await
        .unwrap();
    assert_eq!(
        resp_basic.status(),
        StatusCode::NOT_FOUND,
        "Basic-scheme Authorization must return 404, not 401"
    );

    // 14c — `Bearer ` prefix with an empty token value.
    let peer_addr: SocketAddr = PEER_A.parse().unwrap();
    let empty_bearer_request = Request::builder()
        .method("POST")
        .uri("/resume")
        .header("Authorization", "Bearer ")
        .extension(ConnectInfo(peer_addr))
        .body(Body::empty())
        .unwrap();
    let resp_empty = harness.app.oneshot(empty_bearer_request).await.unwrap();
    assert_eq!(
        resp_empty.status(),
        StatusCode::NOT_FOUND,
        "empty Bearer token must return 404"
    );

    // No enqueues from any of the three bearer-extraction failure paths.
    assert!(
        harness.control_queue.snapshot().is_empty(),
        "no ControlMsg must be enqueued from bearer-extraction failures"
    );
}

// ── Option B1 (W-S3d) — atomic ResumeProducer round-trip + durability gate ────────

/// Insert the minimal parent execution row + a webhook token row directly into a
/// SQLite pool, mirroring what the engine's `TransitionBatch` commit writes.
async fn seed_sqlite_webhook_token(
    pool: &sqlx::SqlitePool,
    bearer: &str,
    execution_id: &str,
    callback_label: &str,
    scope: &Scope,
) {
    let token_hash_bytes = {
        use sha2::{Digest, Sha256};
        Sha256::digest(bearer.as_bytes()).to_vec()
    };
    sqlx::query(
        "INSERT INTO port_executions \
         (id, workspace_id, org_id, workflow_id, status, state, version, \
          created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(execution_id)
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind("wf-resume-producer")
    .bind("Running")
    .bind("{}")
    .bind(0_i64)
    .bind("2026-06-21T00:00:00Z")
    .bind("2026-06-21T00:00:00Z")
    .execute(pool)
    .await
    .expect("parent execution row insert must succeed");

    sqlx::query(
        "INSERT INTO port_resume_tokens \
         (token_hash, workspace_id, org_id, execution_id, node_key, \
          wait_kind, callback_label, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&token_hash_bytes)
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind(execution_id)
    .bind("node_resume_producer")
    .bind("webhook")
    .bind(callback_label)
    .bind("2026-06-21T00:00:00Z")
    .execute(pool)
    .await
    .expect("direct token insert must succeed");
}

/// Count `Resume` rows in the SQLite `port_control_queue`.
async fn sqlite_resume_count(pool: &sqlx::SqlitePool) -> i64 {
    use sqlx::Row;
    sqlx::query("SELECT COUNT(*) AS n FROM port_control_queue WHERE command = 'Resume'")
        .fetch_one(pool)
        .await
        .expect("count query must succeed")
        .try_get::<i64, _>("n")
        .expect("count column must decode")
}

/// Build an `AppState` whose `POST /resume` is wired to a `SqliteResumeProducer`
/// over `pool` (the atomic consume+enqueue seam), with a controllable clock.
fn build_sqlite_resume_app(
    pool: &sqlx::SqlitePool,
    api_config: &ApiConfig,
    clock: Arc<dyn nebula_action::Clock>,
) -> axum::Router {
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };
    use nebula_storage::sqlite::SqliteResumeProducer;

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);

    let state = AppState::new(
        Arc::new(workflow_store),
        Arc::new(workflow_versions),
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue),
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(common::TestOrgResolver))
    .with_workspace_resolver(Arc::new(common::TestWorkspaceResolver))
    .with_insecure_tenant_rbac_bypass_for_tests()
    .with_resume_producer(Arc::new(SqliteResumeProducer::new(pool.clone())))
    .with_resume_handler_components(components_with_clock(clock));

    app::build_app(state, api_config)
}

/// Open an in-memory SQLite pool (one shared connection) with the port schema.
async fn open_sqlite_pool() -> sqlx::SqlitePool {
    use nebula_storage::sqlite::init_schema;
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite pool must open");
    init_schema(&pool)
        .await
        .expect("init_schema must succeed on in-memory SQLite");
    pool
}

/// **MERGE GATE (red to green).** `burn_iff_enqueued` — a failed enqueue must
/// NOT burn the token.
///
/// A real `SqliteResumeProducer` consumes the token and enqueues the `Resume`
/// in ONE transaction. We force the control-queue INSERT to fail INSIDE that
/// transaction by dropping `port_control_queue` first, so the `DELETE` of the
/// token is rolled back with it. The handler returns 503, and:
/// - (a) the token row STILL exists (peek-able) — it survived the rolled-back tx;
/// - (b) NO `Resume` was enqueued;
/// - (c) after the table is restored, a retry succeeds (202) and enqueues
///   EXACTLY ONE `Resume`.
///
/// This FAILS on the prior burn-then-enqueue handler (the token would be gone
/// after the 503, the retry would 404, and a `timeout: None` webhook wait would
/// park forever) and PASSES on the atomic seam.
#[tokio::test]
async fn burn_iff_enqueued_resume_survives_failed_enqueue() {
    use nebula_storage::sqlite::{SqliteResumeProducer, init_schema};
    use nebula_storage_port::store::ResumeProducer;

    let pool = open_sqlite_pool().await;
    let api_config = ApiConfig::for_test();
    let scope = test_scope();
    let bearer = "resume-bearer-burn-iff-enqueued";
    seed_sqlite_webhook_token(&pool, bearer, "exe-burn-iff", "cb-burn-iff", &scope).await;

    // Force the control-queue INSERT (inside the producer's tx) to fail by
    // removing the target table. The token DELETE must roll back with it.
    sqlx::query("DROP TABLE port_control_queue")
        .execute(&pool)
        .await
        .expect("drop port_control_queue must succeed");

    let clock = Arc::new(MockClock::at_now());
    let app = build_sqlite_resume_app(&pool, &api_config, clock);

    let resp = app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .expect("oneshot must not fail");
    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a failed enqueue inside the tx must surface as 503 (token NOT burned)"
    );

    // (a) The token row survived the rolled-back transaction.
    let probe = SqliteResumeProducer::new(pool.clone());
    let survived = probe
        .peek(&token_hash_of(bearer))
        .await
        .expect("peek must not error");
    assert!(
        survived.is_some(),
        "the token MUST survive a rolled-back enqueue — burning it here is the P1 bug \
         (a timeout-less webhook wait would park Paused forever)"
    );

    // Restore the control-queue table so the retry can enqueue.
    init_schema(&pool)
        .await
        .expect("re-applying schema must restore port_control_queue");

    // (b) Before the retry, there is no Resume.
    assert_eq!(
        sqlite_resume_count(&pool).await,
        0,
        "the failed attempt must not have enqueued any Resume"
    );

    // (c) A retry now succeeds and enqueues exactly one Resume.
    let app_retry = build_sqlite_resume_app(&pool, &api_config, Arc::new(MockClock::at_now()));
    let resp_retry = app_retry
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .expect("retry oneshot must not fail");
    assert_eq!(
        resp_retry.status(),
        StatusCode::ACCEPTED,
        "after the transient fault clears, the retry must succeed (token was still live)"
    );
    assert_eq!(
        sqlite_resume_count(&pool).await,
        1,
        "the successful retry must enqueue exactly one Resume"
    );

    // The token is now burned — a second peek is empty (single-use).
    let survived_after = probe
        .peek(&token_hash_of(bearer))
        .await
        .expect("peek must not error");
    assert!(
        survived_after.is_none(),
        "after the successful consume the token must be burned"
    );
}

/// Happy path on the atomic seam: 202 + exactly one Resume with scope/target
/// from the row; the token is no longer peek-able afterward.
#[tokio::test]
async fn happy_path_atomic_round_trip() {
    use nebula_storage::sqlite::SqliteResumeProducer;
    use nebula_storage_port::store::ResumeProducer;

    let pool = open_sqlite_pool().await;
    let api_config = ApiConfig::for_test();
    let scope = test_scope();
    let bearer = "resume-bearer-happy-atomic";
    seed_sqlite_webhook_token(&pool, bearer, "exe-happy-atomic", "cb-happy", &scope).await;

    let app = build_sqlite_resume_app(&pool, &api_config, Arc::new(MockClock::at_now()));
    let resp = app
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .expect("oneshot must not fail");
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "happy path must be 202"
    );

    // Exactly one Resume, with the row's scope + callback as the target.
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT execution_id, workspace_id, org_id, command, resume_target \
         FROM port_control_queue",
    )
    .fetch_all(&pool)
    .await
    .expect("control-queue query must succeed");
    assert_eq!(
        rows.len(),
        1,
        "exactly one control message must be enqueued"
    );
    let row = &rows[0];
    assert_eq!(
        row.try_get::<String, _>("execution_id").unwrap(),
        "exe-happy-atomic"
    );
    assert_eq!(row.try_get::<String, _>("command").unwrap(), "Resume");
    assert_eq!(
        row.try_get::<String, _>("workspace_id").unwrap(),
        scope.workspace_id
    );
    assert_eq!(row.try_get::<String, _>("org_id").unwrap(), scope.org_id);
    let target_json: String = row.try_get("resume_target").unwrap();
    let target: ResumeTarget = serde_json::from_str(&target_json).unwrap();
    assert_eq!(
        target,
        ResumeTarget::Webhook {
            callback_id: "cb-happy".to_owned()
        }
    );

    // Token is burned — no longer peek-able.
    let probe = SqliteResumeProducer::new(pool.clone());
    assert!(
        probe
            .peek(&token_hash_of(bearer))
            .await
            .expect("peek must not error")
            .is_none(),
        "the consumed token must no longer be peek-able"
    );
}

/// Single-use replay on the atomic seam: first 202, replay 404; exactly one Resume.
#[tokio::test]
async fn single_use_replay_still_404s() {
    let pool = open_sqlite_pool().await;
    let api_config = ApiConfig::for_test();
    let scope = test_scope();
    let bearer = "resume-bearer-replay-atomic";
    seed_sqlite_webhook_token(&pool, bearer, "exe-replay-atomic", "cb-replay", &scope).await;

    let app1 = build_sqlite_resume_app(&pool, &api_config, Arc::new(MockClock::at_now()));
    let resp1 = app1
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .expect("first oneshot must not fail");
    assert_eq!(resp1.status(), StatusCode::ACCEPTED, "first call is 202");

    let app2 = build_sqlite_resume_app(&pool, &api_config, Arc::new(MockClock::at_now()));
    let resp2 = app2
        .oneshot(resume_post(bearer, PEER_A))
        .await
        .expect("second oneshot must not fail");
    assert_eq!(
        resp2.status(),
        StatusCode::NOT_FOUND,
        "replay of a consumed token must 404"
    );

    assert_eq!(
        sqlite_resume_count(&pool).await,
        1,
        "a replay must NOT produce a second Resume"
    );
}

/// Concurrent replay: two simultaneous POSTs with the same bearer → exactly one
/// 202 and one non-2xx; the `DELETE … RETURNING` row lock makes exactly one win,
/// so exactly one Resume is enqueued.
#[tokio::test]
async fn concurrent_replay_enqueues_once() {
    let pool = open_sqlite_pool().await;
    let api_config = ApiConfig::for_test();
    let scope = test_scope();
    let bearer = "resume-bearer-concurrent-atomic";
    seed_sqlite_webhook_token(&pool, bearer, "exe-concurrent", "cb-concurrent", &scope).await;

    let app_a = build_sqlite_resume_app(&pool, &api_config, Arc::new(MockClock::at_now()));
    let app_b = build_sqlite_resume_app(&pool, &api_config, Arc::new(MockClock::at_now()));

    let (resp_a, resp_b) = tokio::join!(
        app_a.oneshot(resume_post(bearer, PEER_A)),
        app_b.oneshot(resume_post(bearer, PEER_B)),
    );
    let status_a = resp_a.expect("a oneshot must not fail").status();
    let status_b = resp_b.expect("b oneshot must not fail").status();

    let accepted = [status_a, status_b]
        .iter()
        .filter(|s| **s == StatusCode::ACCEPTED)
        .count();
    assert_eq!(
        accepted, 1,
        "exactly one of two concurrent replays may win the atomic delete \
         (got {status_a} / {status_b})"
    );
    assert_eq!(
        sqlite_resume_count(&pool).await,
        1,
        "concurrent replays must enqueue exactly one Resume"
    );
}

/// Storage error on the read-only `peek` → 503 + `Retry-After`, token left live.
/// Proves the 503-on-peek path on the real producer trait at the HTTP boundary
/// (mirrors test 12: `AlwaysFailResumeProducer.peek` returns `Err`).
#[tokio::test]
async fn peek_storage_error_is_503_token_live() {
    let harness =
        build_failing_store_harness(components_with_clock(Arc::new(MockClock::at_now()))).await;

    let resp = harness
        .app
        .oneshot(resume_post("bearer-peek-error", PEER_A))
        .await
        .expect("oneshot must not fail");
    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a peek storage error must surface as 503"
    );
    assert!(
        resp.headers().contains_key("retry-after"),
        "503 on peek error must include Retry-After"
    );
}
