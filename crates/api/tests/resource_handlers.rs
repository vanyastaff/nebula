//! Integration tests for the workspace-scoped resource catalog handlers.
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` and
//! `GET .../resources/{res}` are config-CRUD read endpoints (ADR-0047
//! Stub Endpoint Policy retired for these routes). These tests boot the
//! in-memory app with a fake [`ResourceRepo`] and assert the honest
//! `ResourceEntry` → `ResourceSummary` mapping: `res_<ULID>` id encoding,
//! `display_name` → `name`, soft-deleted rows excluded, and no raw
//! `config` ever surfaced (ADR-0028 §7).
//!
//! The single-resource read additionally enforces tenant isolation: a
//! resource whose `workspace_id` differs from the caller's authorized
//! workspace is **404 Not Found**, indistinguishable from a missing row
//! (no cross-tenant existence or content leak — `ResourceRepo::get` is
//! looked up purely by id and is *not* workspace-scoped, so the handler
//! is the isolation boundary). A soft-deleted row and an unparsable id
//! are likewise 404.

mod common;

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use nebula_api::{ApiConfig, AppState, app};
use nebula_core::{ResourceId, WorkspaceId};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo,
    repos::{InMemoryControlQueueRepo, ResourceEntry, ResourceRepo},
};
use tower::ServiceExt;

/// Raw 16-byte workspace id matching [`TEST_WS`], as the repo stores it.
fn test_ws_bytes() -> Vec<u8> {
    TEST_WS
        .parse::<WorkspaceId>()
        .expect("TEST_WS is a valid ws_<ULID>")
        .as_bytes()
        .to_vec()
}

/// Build a `ResourceEntry` whose id is the 16 raw bytes of `id`, owned by
/// `workspace_id_bytes`.
///
/// Grounding the stored id bytes in a real [`ResourceId`] (rather than
/// arbitrary filler bytes) makes the path-id ↔ stored-id decode round-trip
/// exact: a 404 from the read handlers can then only come from their
/// isolation / soft-delete filters, never an id-encoding mismatch. The
/// `config` is deliberately secret-shaped so every read path is asserted
/// to never echo it (ADR-0028 §7).
fn entry(
    id: ResourceId,
    workspace_id_bytes: Vec<u8>,
    slug: &str,
    display_name: &str,
    kind: &str,
    version: i64,
    deleted: bool,
) -> ResourceEntry {
    ResourceEntry {
        id: id.as_bytes().to_vec(),
        workspace_id: workspace_id_bytes,
        slug: slug.to_owned(),
        display_name: display_name.to_owned(),
        kind: kind.to_owned(),
        // A config value that must NEVER appear in any response body.
        config: serde_json::json!({ "secret_looking_key": "do-not-leak" }),
        created_at: chrono::Utc::now(),
        created_by: vec![0u8; 16],
        version,
        deleted_at: deleted.then_some(chrono::Utc::now()),
    }
}

/// One parametrizable fake [`ResourceRepo`] for every resource-handler
/// test.
///
/// - `list` returns `list_rows` verbatim (including soft-deleted rows) —
///   the handler owns the `deleted_at` filter, so the fixture must not
///   pre-filter. When `expect_list_ws` is set the looked-up workspace id
///   is asserted, pinning that the handler scopes `list()` to the tenant.
/// - `get` mirrors the real id-keyed store: it returns `get_row` only
///   when the looked-up id equals the stored row's id, and is *not*
///   workspace-scoped — tenant isolation is the handler's job, asserted
///   by the get tests.
/// - `create` / `update` / `soft_delete` record their argument into a
///   capture `Mutex` (and `Ok`), so a mutation test can inspect exactly
///   what the handler persisted (e.g. that `workspace_id` is the
///   caller's, never client-supplied).
#[derive(Default)]
struct FakeResourceRepo {
    list_rows: Vec<ResourceEntry>,
    get_row: Option<ResourceEntry>,
    expect_list_ws: Option<Vec<u8>>,
    last_create: Mutex<Option<ResourceEntry>>,
    // Populated by `update` / `soft_delete`; read by the mutation tests
    // that exercise those handlers (the accessors land with them).
    #[allow(
        dead_code,
        reason = "shared mutation-capture fixture; create is the first consumer"
    )]
    last_update: Mutex<Option<(ResourceEntry, i64)>>,
    #[allow(
        dead_code,
        reason = "shared mutation-capture fixture; create is the first consumer"
    )]
    last_soft_delete: Mutex<Option<Vec<u8>>>,
}

impl FakeResourceRepo {
    /// Repo that serves `rows` from `list` and asserts `list()` is scoped
    /// to the test workspace.
    fn with_list(rows: Vec<ResourceEntry>) -> Self {
        Self {
            list_rows: rows,
            expect_list_ws: Some(test_ws_bytes()),
            ..Self::default()
        }
    }

    /// Repo whose `get(id)` resolves `row` (by exact id match), faithfully
    /// returning even a foreign-workspace row so the handler is the
    /// isolation boundary.
    fn with_get(row: Option<ResourceEntry>) -> Self {
        Self {
            get_row: row,
            ..Self::default()
        }
    }

    /// The `ResourceEntry` the handler passed to `create`, if any.
    ///
    /// Capture-inspection accessor used by the create tests. The
    /// `last_update` / `last_soft_delete` siblings are populated by the
    /// same fixture; their accessors are added alongside the handlers
    /// that exercise them.
    fn created(&self) -> Option<ResourceEntry> {
        self.last_create
            .lock()
            .expect("create capture mutex not poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl ResourceRepo for FakeResourceRepo {
    async fn create(&self, r: &ResourceEntry) -> Result<(), nebula_storage::StorageError> {
        *self
            .last_create
            .lock()
            .expect("create capture mutex not poisoned") = Some(r.clone());
        Ok(())
    }

    async fn get(&self, id: &[u8]) -> Result<Option<ResourceEntry>, nebula_storage::StorageError> {
        Ok(self
            .get_row
            .as_ref()
            .filter(|e| e.id.as_slice() == id)
            .cloned())
    }

    async fn get_by_slug(
        &self,
        _ws: &[u8],
        _slug: &str,
    ) -> Result<Option<ResourceEntry>, nebula_storage::StorageError> {
        Ok(None)
    }

    async fn update(
        &self,
        r: &ResourceEntry,
        expected: i64,
    ) -> Result<(), nebula_storage::StorageError> {
        *self
            .last_update
            .lock()
            .expect("update capture mutex not poisoned") = Some((r.clone(), expected));
        Ok(())
    }

    async fn soft_delete(&self, id: &[u8]) -> Result<(), nebula_storage::StorageError> {
        *self
            .last_soft_delete
            .lock()
            .expect("soft_delete capture mutex not poisoned") = Some(id.to_vec());
        Ok(())
    }

    async fn list(
        &self,
        workspace_id: &[u8],
    ) -> Result<Vec<ResourceEntry>, nebula_storage::StorageError> {
        if let Some(expected) = &self.expect_list_ws {
            assert_eq!(
                workspace_id,
                expected.as_slice(),
                "handler must scope list() to the tenant workspace id bytes"
            );
        }
        Ok(self.list_rows.clone())
    }
}

/// Build an [`AppState`] wired with `repo` as the resource catalog
/// backend (plus the shared org/workspace resolvers).
fn state_with_repo(api_config: &ApiConfig, repo: Arc<dyn ResourceRepo>) -> AppState {
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());

    AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
    .with_resource_repo(repo)
}

#[tokio::test]
async fn list_resources_returns_200_with_mapped_summaries() {
    let api_config = ApiConfig::for_test();
    // Two live rows plus one soft-deleted one for the test workspace;
    // `list` returns every row (the handler owns the soft-delete filter).
    let rows = vec![
        entry(
            ResourceId::new(),
            test_ws_bytes(),
            "http-pool",
            "HTTP Pool",
            "http_pool",
            3,
            false,
        ),
        entry(
            ResourceId::new(),
            test_ws_bytes(),
            "redis",
            "Redis Cache",
            "redis_cache",
            1,
            false,
        ),
        entry(
            ResourceId::new(),
            test_ws_bytes(),
            "old-pool",
            "Deleted Pool",
            "http_pool",
            7,
            true,
        ),
    ];
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_list(rows));
    let state = state_with_repo(&api_config, repo);
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/resources"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Honesty contract: the route is implemented, NOT a 501 stub.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "list_resources must be implemented (200), not the ADR-0047 501 stub"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8(body.to_vec()).expect("body is utf-8");

    // Raw, non-secret config must never leak into the summary payload.
    assert!(
        !raw.contains("secret_looking_key") && !raw.contains("do-not-leak"),
        "response must not surface raw resource config; body: {raw}"
    );

    let json: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON body");
    let resources = json["resources"]
        .as_array()
        .expect("`resources` must be a JSON array");

    // The soft-deleted row is excluded → exactly 2 live summaries.
    assert_eq!(
        resources.len(),
        2,
        "soft-deleted resources must be excluded; got {resources:?}"
    );

    let first = &resources[0];
    let id = first["id"].as_str().expect("id is a string");
    assert!(
        id.starts_with("res_"),
        "id must be the prefixed `res_<ULID>` encoding, got {id}"
    );
    assert_eq!(first["name"], "HTTP Pool", "name must be display_name");
    assert_eq!(first["kind"], "http_pool");
    assert_eq!(first["version"], "3", "version must render the i64 version");
    assert_eq!(
        first["attached_to_workflows"]
            .as_array()
            .expect("attached_to_workflows is an array")
            .len(),
        0,
        "workflow attachment is not tracked yet — must be honestly empty"
    );

    let second = &resources[1];
    assert_eq!(second["name"], "Redis Cache");
    assert_eq!(second["kind"], "redis_cache");
    assert_eq!(second["version"], "1");
}

#[tokio::test]
async fn list_resources_without_repo_is_service_unavailable() {
    // No `.with_resource_repo(...)` → the optional dependency is None.
    // Mirrors the action/plugin catalog convention: 503, not a 501 stub.
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::for_test();
    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver));

    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/resources"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured resource repo must be 503 (not the retired 501 stub)"
    );
}

// ── create_resource ──────────────────────────────────────────────────────────
//
// The schema/closed-set validation *logic* (schema-valid ⇒ ok,
// schema-invalid ⇒ Register, secret-shaped field ⇒ Register without
// leaking the value, unknown kind ⇒ UnknownKind, NO Manager mutation) is
// pinned at the engine layer in
// `crates/engine/tests/resource_registrar_validate.rs` — that is where
// `ResourceRegistrarRegistry::validate` can be driven against a real
// `R::Config` schema. The API layer must not depend on `nebula-resource`
// (deny.toml `[[wrappers]]`: "no upward deps from API"), so it cannot
// build a populated registrar here. These tests therefore pin the
// *handler's HTTP contract and tenant isolation* over every path
// reachable without a concrete resource type: unknown-kind → 409,
// no-validation-backend → 422 (fail closed), no-repo → 503, and the
// structural impossibility of a client-supplied owning workspace.

/// POST a create body to the resources collection.
async fn create_resource_request(
    app: axum::Router,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    let token = create_test_jwt();
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(ws_path("/resources"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("x-csrf-token", TEST_CSRF_TOKEN)
            .header("cookie", TEST_CSRF_COOKIE)
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

/// A well-formed create body for an arbitrary kind.
fn create_body(kind: &str) -> serde_json::Value {
    serde_json::json!({
        "slug": "my-pool",
        "display_name": "My Pool",
        "kind": kind,
        "config": { "base_url": "https://api.example.com" },
    })
}

/// An empty closed allowlist rejects *every* kind as `UnknownKind`,
/// which the handler maps to **409 Conflict** (the codebase's Conflict
/// status — `RegistrarError::UnknownKind` → `ErrorCategory::Conflict`).
/// Crucially the row is NOT persisted: validation gates persistence.
#[tokio::test]
async fn create_resource_unknown_kind_is_409_and_not_persisted() {
    let api_config = ApiConfig::for_test();
    let repo = Arc::new(FakeResourceRepo::default());
    let registrars = Arc::new(nebula_engine::ResourceRegistrarRegistry::new());
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>)
        .with_resource_registrars(registrars);
    let app = app::build_app(state, &api_config);

    let response = create_resource_request(app, create_body("never_registered_kind")).await;
    let status = response.status();
    let raw = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .expect("body is utf-8");

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an unknown resource kind must be 409 (RegistrarError::UnknownKind \
         → Conflict), got {status}, body: {raw}"
    );
    assert!(
        repo.created().is_none(),
        "a config whose kind failed validation must NOT be persisted"
    );
}

/// With no validation backend wired, the handler fails **closed**: 422,
/// and nothing is persisted. Persisting an unvalidated config (which
/// could carry an inlined secret) is never acceptable (ADR-0028 §7).
#[tokio::test]
async fn create_resource_without_validation_backend_is_422_fail_closed() {
    let api_config = ApiConfig::for_test();
    let repo = Arc::new(FakeResourceRepo::default());
    // `.with_resource_registrars(...)` deliberately NOT called.
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = create_resource_request(app, create_body("http_pool")).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "no validation backend ⇒ fail closed with 422, never persist unvalidated"
    );
    assert!(
        repo.created().is_none(),
        "an unvalidated config must NEVER be persisted"
    );
}

/// No resource repo configured ⇒ 503 (same convention as the read
/// endpoints), checked before any validation work.
#[tokio::test]
async fn create_resource_without_repo_is_503() {
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::for_test();
    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver));
    let app = app::build_app(state, &api_config);

    let response = create_resource_request(app, create_body("http_pool")).await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured resource repo must be 503"
    );
}

/// SECURITY (tenant isolation / confused-deputy): the create body has
/// **no** workspace/owner field, so a client cannot target another
/// tenant's workspace. A request that *attempts* to smuggle a
/// `workspace_id` (and even a foreign-looking one) is accepted as the
/// `CreateResourceRequest` shape with that extra key ignored — and since
/// the kind here is unknown the request is rejected (409) and nothing is
/// persisted, so no foreign-workspace row can ever be written. The DTO
/// shape itself is the structural guarantee; this pins it cannot
/// regress into honoring a body-supplied tenant.
#[tokio::test]
async fn create_resource_request_body_cannot_set_workspace() {
    use nebula_api::models::CreateResourceRequest;

    // A body that tries to smuggle a foreign workspace + owner. These
    // extra keys are not part of `CreateResourceRequest`, so serde drops
    // them: the deserialized request carries only slug/display_name/
    // kind/config — there is no field through which a caller could pick
    // the owning workspace.
    let smuggled = serde_json::json!({
        "slug": "evil",
        "display_name": "Evil Pool",
        "kind": "http_pool",
        "config": { "base_url": "https://x.test" },
        "workspace_id": "ws_99999999999999999999999999",
        "created_by": "usr_99999999999999999999999999",
    });
    let parsed: CreateResourceRequest =
        serde_json::from_value(smuggled.clone()).expect("extra keys are ignored by the DTO");
    assert_eq!(parsed.slug, "evil");
    assert_eq!(parsed.kind, "http_pool");
    // (Compile-time guarantee: `CreateResourceRequest` has no
    // `workspace_id` / owner field at all — the only way to express
    // "which workspace" is the authenticated path context.)

    // End to end: even attempting the smuggle cannot persist a foreign
    // row — the (unknown) kind fails validation first (409), so the
    // create never reaches the repo regardless of the body.
    let api_config = ApiConfig::for_test();
    let repo = Arc::new(FakeResourceRepo::default());
    let registrars = Arc::new(nebula_engine::ResourceRegistrarRegistry::new());
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>)
        .with_resource_registrars(registrars);
    let app = app::build_app(state, &api_config);

    let response = create_resource_request(app, smuggled).await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "unknown kind is still rejected; a smuggled workspace_id changes nothing"
    );
    assert!(
        repo.created().is_none(),
        "no row — and therefore no foreign-workspace row — may be persisted"
    );
}

// ── get_resource ─────────────────────────────────────────────────────────────

/// Build a single-row get fixture state: `get(id)` resolves this entry by
/// exact id match (and is *not* workspace-scoped — the handler is the
/// tenant-isolation boundary). A fixed `"the-slug"` keeps the get tests'
/// call sites terse.
fn get_state(api_config: &ApiConfig, row: Option<ResourceEntry>) -> AppState {
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(row));
    state_with_repo(api_config, repo)
}

/// `entry` with the fixed slug the single-resource read tests use.
fn get_entry(
    id: ResourceId,
    workspace_id_bytes: Vec<u8>,
    display_name: &str,
    kind: &str,
    version: i64,
    deleted: bool,
) -> ResourceEntry {
    entry(
        id,
        workspace_id_bytes,
        "the-slug",
        display_name,
        kind,
        version,
        deleted,
    )
}

async fn get_resource_request(app: axum::Router, res_id: &str) -> axum::http::Response<Body> {
    let token = create_test_jwt();
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(ws_path(&format!("/resources/{res_id}")))
            .header("authorization", format!("Bearer {token}"))
            .header("x-csrf-token", TEST_CSRF_TOKEN)
            .header("cookie", TEST_CSRF_COOKIE)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn get_resource_returns_200_with_mapped_summary() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let row = get_entry(id, test_ws_bytes(), "HTTP Pool", "http_pool", 3, false);
    let state = get_state(&api_config, Some(row));
    let app = app::build_app(state, &api_config);

    let response = get_resource_request(app, &id.to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a live resource in the caller's workspace must be 200"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8(body.to_vec()).expect("body is utf-8");

    // Raw config must never leak into the single-resource payload.
    assert!(
        !raw.contains("secret_looking_key") && !raw.contains("do-not-leak"),
        "response must not surface raw resource config; body: {raw}"
    );

    let json: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON body");
    let returned_id = json["id"].as_str().expect("id is a string");
    assert_eq!(
        returned_id,
        id.to_string(),
        "id must round-trip to the same `res_<ULID>` string"
    );
    assert!(
        returned_id.starts_with("res_"),
        "id must be the prefixed `res_<ULID>` encoding, got {returned_id}"
    );
    assert_eq!(json["name"], "HTTP Pool", "name must be display_name");
    assert_eq!(json["kind"], "http_pool");
    assert_eq!(json["version"], "3", "version must render the i64 version");
    assert_eq!(
        json["attached_to_workflows"]
            .as_array()
            .expect("attached_to_workflows is an array")
            .len(),
        0,
        "workflow attachment is not tracked yet — must be honestly empty"
    );
}

#[tokio::test]
async fn get_resource_unknown_id_is_404() {
    let api_config = ApiConfig::for_test();
    // Repo has no entry at all → any well-formed id is unknown.
    let state = get_state(&api_config, None);
    let app = app::build_app(state, &api_config);

    let response = get_resource_request(app, &ResourceId::new().to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an id with no backing row must be 404"
    );
}

/// SECURITY (tenant isolation): a resource that *exists* but whose
/// `workspace_id` is a different workspace than the caller's authorized
/// path workspace MUST be **404** — not 200 (content leak) and not 403
/// (existence leak). `ResourceRepo::get` is keyed purely by id, so the
/// fake faithfully returns the foreign-workspace row; the handler is the
/// isolation boundary and must filter it as if it does not exist.
#[tokio::test]
async fn get_resource_cross_workspace_is_404_not_leaked() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();

    // 16 deterministic bytes that are NOT the caller's TEST_WS workspace
    // id — a genuinely different tenant. Sanity-check the premise so the
    // test cannot silently degrade into "same workspace".
    let other_workspace = vec![0xAB_u8; 16];
    assert_ne!(
        other_workspace,
        test_ws_bytes(),
        "the cross-workspace fixture must differ from the caller's workspace"
    );

    // A live (NOT soft-deleted) resource — so a 404 can only be caused by
    // the workspace mismatch, never by the soft-delete filter.
    let foreign = get_entry(id, other_workspace, "Foreign Pool", "http_pool", 9, false);
    let state = get_state(&api_config, Some(foreign));
    let app = app::build_app(state, &api_config);

    let response = get_resource_request(app, &id.to_string()).await;
    let status = response.status();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8(body.to_vec()).expect("body is utf-8");

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a resource in another workspace must be 404 (no cross-tenant \
         read; no existence leak) — got {status}, body: {raw}"
    );
    // Defense in depth: the foreign row's content must not leak even
    // partially through the error body.
    assert!(
        !raw.contains("Foreign Pool")
            && !raw.contains("secret_looking_key")
            && !raw.contains("do-not-leak"),
        "cross-workspace 404 must not echo the foreign resource's \
         content; body: {raw}"
    );
}

#[tokio::test]
async fn get_resource_soft_deleted_is_404() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    // Belongs to the caller's workspace but is a tombstone (deleted_at set).
    let tombstone = get_entry(id, test_ws_bytes(), "Deleted Pool", "http_pool", 7, true);
    let state = get_state(&api_config, Some(tombstone));
    let app = app::build_app(state, &api_config);

    let response = get_resource_request(app, &id.to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a soft-deleted resource is a tombstone, not a resource → 404"
    );
}

#[tokio::test]
async fn get_resource_malformed_id_is_404() {
    let api_config = ApiConfig::for_test();
    let state = get_state(&api_config, None);
    let app = app::build_app(state, &api_config);

    // Not a `res_<ULID>` — an unparsable id is "not found", consistent
    // with the other workspace-scoped get-by-id handlers.
    let response = get_resource_request(app, "not-a-valid-resource-id").await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an unparsable resource id must be 404, never 500"
    );
}
