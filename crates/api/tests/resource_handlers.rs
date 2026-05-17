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
use nebula_core::{ResourceId, ResourceKey, WorkspaceId};
use nebula_engine::{EngineResourceStatus, ResourceRuntimeStatus};
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
    // that exercise those handlers via `updated()` / `soft_deleted()`.
    last_update: Mutex<Option<(ResourceEntry, i64)>>,
    last_soft_delete: Mutex<Option<Vec<u8>>>,
    // When set, `update` returns this CAS conflict instead of `Ok` — so a
    // stale-`expected_version` test can drive the handler's
    // `StorageError::Conflict` → 409 mapping without a stateful store.
    update_conflict: Option<(i64, i64)>,
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

    /// Repo whose `get(id)` resolves `row` and whose `update` rejects
    /// with a CAS [`StorageError::Conflict`] (`expected` vs `actual`)
    /// instead of `Ok`. Drives the handler's stale-version → 409 path
    /// without a stateful store.
    fn with_get_and_update_conflict(row: ResourceEntry, expected: i64, actual: i64) -> Self {
        Self {
            get_row: Some(row),
            update_conflict: Some((expected, actual)),
            ..Self::default()
        }
    }

    /// The `ResourceEntry` the handler passed to `create`, if any.
    fn created(&self) -> Option<ResourceEntry> {
        self.last_create
            .lock()
            .expect("create capture mutex not poisoned")
            .clone()
    }

    /// The `(entry, expected_version)` the handler passed to `update`, if
    /// any. `None` proves `update` was never reached (e.g. a foreign-
    /// workspace target must collapse to 404 *before* any mutation).
    fn updated(&self) -> Option<(ResourceEntry, i64)> {
        self.last_update
            .lock()
            .expect("update capture mutex not poisoned")
            .clone()
    }

    /// The raw id bytes the handler passed to `soft_delete`, if any.
    /// `None` proves `soft_delete` was never reached.
    fn soft_deleted(&self) -> Option<Vec<u8>> {
        self.last_soft_delete
            .lock()
            .expect("soft_delete capture mutex not poisoned")
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
        // Capture first (even on the conflict path): a CAS-conflict test
        // still asserts the handler reached `update` with the EXISTING
        // row's workspace_id, never a body-supplied one.
        *self
            .last_update
            .lock()
            .expect("update capture mutex not poisoned") = Some((r.clone(), expected));
        if let Some((expected_v, actual_v)) = self.update_conflict {
            return Err(nebula_storage::StorageError::conflict(
                "resource", "res", expected_v, actual_v,
            ));
        }
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

// ── update_resource (CAS) ────────────────────────────────────────────────────
//
// Same constraint as the create tests: the schema/closed-set validation
// *logic* against a real `R::Config` is pinned at the engine layer
// (`crates/engine/tests/resource_registrar_validate.rs`). The API layer
// must not depend on `nebula-resource` (deny.toml `[[wrappers]]`), so
// these tests pin the *handler's HTTP contract, CAS mapping, and tenant
// isolation* over every path reachable without a concrete resource type:
// re-validation fires before persistence (unknown-kind → 409,
// no-validation-backend → 422 fail closed), the CAS-mismatch
// `StorageError::Conflict` maps to 409, the persisted row keeps the
// EXISTING workspace_id (never body-supplied), a foreign / unknown / soft-
// deleted target collapses to an indistinguishable 404 with NO mutation,
// and no-repo → 503.

/// Issue a `PUT .../resources/{res}` with `body`.
async fn update_resource_request(
    app: axum::Router,
    res_id: &str,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    let token = create_test_jwt();
    app.oneshot(
        Request::builder()
            .method("PUT")
            .uri(ws_path(&format!("/resources/{res_id}")))
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

/// A well-formed update body. `config` is deliberately secret-shaped so
/// every rejection path is asserted never to echo it (ADR-0028 §7).
fn update_body(kind: &str, expected_version: i64) -> serde_json::Value {
    serde_json::json!({
        "display_name": "Renamed Pool",
        "kind": kind,
        "config": { "rotated_secret_key": "do-not-leak-on-update" },
        "expected_version": expected_version,
    })
}

/// SECURITY (cross-tenant mutation isolation — the severe surface): a
/// resource that exists but is owned by ANOTHER workspace MUST collapse
/// to a 404 *before any mutation*. `ResourceRepo::get`/`update` are keyed
/// purely by id (not workspace-scoped), so the fake faithfully resolves
/// the foreign row; the handler is the isolation boundary. The capture
/// MUST stay `None` — the foreign row is never written, and neither its
/// content nor its existence leaks.
#[tokio::test]
async fn update_resource_cross_workspace_is_404_no_mutation() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();

    let other_workspace = vec![0xAB_u8; 16];
    assert_ne!(
        other_workspace,
        test_ws_bytes(),
        "the cross-workspace fixture must differ from the caller's workspace"
    );
    let foreign = get_entry(id, other_workspace, "Foreign Pool", "http_pool", 4, false);

    let repo = Arc::new(FakeResourceRepo::with_get(Some(foreign)));
    let registrars = Arc::new(nebula_engine::ResourceRegistrarRegistry::new());
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>)
        .with_resource_registrars(registrars);
    let app = app::build_app(state, &api_config);

    let response = update_resource_request(app, &id.to_string(), update_body("http_pool", 4)).await;
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
        StatusCode::NOT_FOUND,
        "a resource in another workspace must be 404 (no cross-tenant \
         mutation; no existence leak) — got {status}, body: {raw}"
    );
    assert!(
        repo.updated().is_none(),
        "the foreign-workspace row must NEVER be passed to update — \
         isolation must short-circuit before any mutation"
    );
    assert!(
        !raw.contains("Foreign Pool")
            && !raw.contains("rotated_secret_key")
            && !raw.contains("do-not-leak"),
        "cross-workspace 404 must not echo the foreign row or the \
         submitted config; body: {raw}"
    );
}

/// An update targeting an id with no backing row is an indistinguishable
/// 404, and nothing is mutated.
#[tokio::test]
async fn update_resource_unknown_id_is_404_no_mutation() {
    let api_config = ApiConfig::for_test();
    let repo = Arc::new(FakeResourceRepo::with_get(None));
    let registrars = Arc::new(nebula_engine::ResourceRegistrarRegistry::new());
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>)
        .with_resource_registrars(registrars);
    let app = app::build_app(state, &api_config);

    let response = update_resource_request(
        app,
        &ResourceId::new().to_string(),
        update_body("http_pool", 0),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an id with no backing row must be 404 on update"
    );
    assert!(
        repo.updated().is_none(),
        "an absent target must not reach update"
    );
}

/// A soft-deleted (tombstoned) row is not a resource: an update to it is
/// 404, and the tombstone is never re-written.
#[tokio::test]
async fn update_resource_soft_deleted_is_404_no_mutation() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let tombstone = get_entry(id, test_ws_bytes(), "Deleted Pool", "http_pool", 7, true);

    let repo = Arc::new(FakeResourceRepo::with_get(Some(tombstone)));
    let registrars = Arc::new(nebula_engine::ResourceRegistrarRegistry::new());
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>)
        .with_resource_registrars(registrars);
    let app = app::build_app(state, &api_config);

    let response = update_resource_request(app, &id.to_string(), update_body("http_pool", 7)).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a soft-deleted resource is a tombstone, not a resource → 404"
    );
    assert!(
        repo.updated().is_none(),
        "a tombstone must not be re-written by update"
    );
}

/// An unparsable id cannot name a resource → 404 (indistinguishable from
/// missing/foreign), never 400/500.
#[tokio::test]
async fn update_resource_malformed_id_is_404() {
    let api_config = ApiConfig::for_test();
    let repo = Arc::new(FakeResourceRepo::with_get(None));
    let registrars = Arc::new(nebula_engine::ResourceRegistrarRegistry::new());
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>)
        .with_resource_registrars(registrars);
    let app = app::build_app(state, &api_config);

    let response =
        update_resource_request(app, "not-a-valid-resource-id", update_body("http_pool", 0)).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an unparsable resource id must be 404 on update, never 500"
    );
}

/// Re-validation fires BEFORE persistence: an unknown `kind` on a row the
/// caller legitimately owns is rejected as 409 (RegistrarError::UnknownKind
/// → Conflict) and the row is NEVER passed to `update`. A PUT must not be
/// a path to persist an unknown-kind config that create would reject.
#[tokio::test]
async fn update_resource_unknown_kind_is_409_no_mutation() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    // A live row the caller owns — so a non-200 can only be the
    // re-validation gate, not isolation/soft-delete.
    let row = get_entry(id, test_ws_bytes(), "My Pool", "http_pool", 2, false);

    let repo = Arc::new(FakeResourceRepo::with_get(Some(row)));
    // Empty registry ⇒ every kind is UnknownKind.
    let registrars = Arc::new(nebula_engine::ResourceRegistrarRegistry::new());
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>)
        .with_resource_registrars(registrars);
    let app = app::build_app(state, &api_config);

    let response = update_resource_request(
        app,
        &id.to_string(),
        update_body("never_registered_kind", 2),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "an unknown resource kind on update must be 409 (UnknownKind → \
         Conflict), re-validated before any write"
    );
    assert!(
        repo.updated().is_none(),
        "a config whose kind failed re-validation must NOT be persisted"
    );
}

/// With no validation backend wired, update fails **closed**: 422, and
/// nothing is persisted — exactly the create-time fail-closed rule. A PUT
/// must never be a path to persist an unvalidated (possibly secret-
/// carrying) config. The error body must not echo the submitted config.
#[tokio::test]
async fn update_resource_without_validation_backend_is_422_fail_closed_no_secret() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let row = get_entry(id, test_ws_bytes(), "My Pool", "http_pool", 1, false);

    let repo = Arc::new(FakeResourceRepo::with_get(Some(row)));
    // `.with_resource_registrars(...)` deliberately NOT called.
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = update_resource_request(app, &id.to_string(), update_body("http_pool", 1)).await;
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
        StatusCode::UNPROCESSABLE_ENTITY,
        "no validation backend ⇒ fail closed with 422 on update, never \
         persist unvalidated; got {status}, body: {raw}"
    );
    assert!(
        repo.updated().is_none(),
        "an unvalidated config must NEVER be persisted on update"
    );
    assert!(
        !raw.contains("rotated_secret_key") && !raw.contains("do-not-leak"),
        "a re-validation rejection must not echo the submitted config / \
         secret values; body: {raw}"
    );
}

/// Gate ordering: re-validation runs BEFORE the CAS write. The repo stub
/// is armed to reject the write with a CAS `StorageError::Conflict`, but
/// the validation backend is absent — so the request must fail closed
/// with 422 (an unvalidated config never even reaches the CAS layer)
/// rather than surface the repo's 409. The CAS→409 mapping itself is
/// asserted directly at the type level in
/// `storage_cas_conflict_maps_to_409_other_storage_errors_stay_500`
/// (the API crate cannot stand up a real registrar — no `nebula-resource`
/// dependency — so the success/CAS-409 e2e path is not constructible
/// here; the type-level test is the decisive coverage).
#[tokio::test]
async fn update_resource_revalidation_precedes_cas() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let row = get_entry(id, test_ws_bytes(), "My Pool", "http_pool", 5, false);

    // Repo would reject with a CAS conflict — but re-validation (absent
    // backend → 422) MUST run first, proving the gate ordering.
    let repo = Arc::new(FakeResourceRepo::with_get_and_update_conflict(row, 5, 9));
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = update_resource_request(app, &id.to_string(), update_body("http_pool", 5)).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "re-validation (no backend → 422) must precede the CAS write — \
         an unvalidated config must never even reach the CAS layer"
    );
    assert!(
        repo.updated().is_none(),
        "update must not be reached when re-validation fails closed"
    );
}

/// No resource repo configured ⇒ 503 (same convention as the read/create
/// endpoints), checked before any validation or isolation work.
#[tokio::test]
async fn update_resource_without_repo_is_503() {
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

    let response = update_resource_request(
        app,
        &ResourceId::new().to_string(),
        update_body("http_pool", 0),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured resource repo must be 503 on update"
    );
}

/// SECURITY (no client-controlled workspace_id): `UpdateResourceRequest`
/// has no workspace/owner field, so a body that tries to smuggle one is
/// accepted with the extra key dropped by serde — there is no field
/// through which a caller could re-home the resource. The persisted
/// `workspace_id` is always the existing row's (== caller ws, verified).
#[tokio::test]
async fn update_resource_request_body_cannot_set_workspace() {
    use nebula_api::models::UpdateResourceRequest;

    let smuggled = serde_json::json!({
        "display_name": "Evil Rename",
        "kind": "http_pool",
        "config": { "k": "v" },
        "expected_version": 3,
        "workspace_id": "ws_99999999999999999999999999",
        "created_by": "usr_99999999999999999999999999",
    });
    let parsed: UpdateResourceRequest =
        serde_json::from_value(smuggled).expect("extra keys are ignored by the DTO");
    assert_eq!(parsed.display_name, "Evil Rename");
    assert_eq!(parsed.kind, "http_pool");
    assert_eq!(parsed.expected_version, 3);
    // (Compile-time guarantee: `UpdateResourceRequest` has no
    // `workspace_id` / owner field at all — the only "which workspace" is
    // the authenticated path context, and on update the persisted row
    // keeps the existing row's workspace_id.)
}

// ── delete_resource (soft) ───────────────────────────────────────────────────

async fn delete_resource_request(app: axum::Router, res_id: &str) -> axum::http::Response<Body> {
    let token = create_test_jwt();
    app.oneshot(
        Request::builder()
            .method("DELETE")
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

/// A live resource the caller owns is soft-deleted: **204 No Content**,
/// and the handler passed the resolved row's id bytes to `soft_delete`.
#[tokio::test]
async fn delete_resource_valid_is_204_and_soft_deletes() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let row = get_entry(id, test_ws_bytes(), "My Pool", "http_pool", 3, false);

    let repo = Arc::new(FakeResourceRepo::with_get(Some(row)));
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = delete_resource_request(app, &id.to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "a live resource the caller owns must soft-delete with 204"
    );
    assert_eq!(
        repo.soft_deleted().as_deref(),
        Some(id.as_bytes().as_slice()),
        "soft_delete must be called with the resolved row's id bytes"
    );
}

/// SECURITY (cross-tenant delete isolation): a resource owned by ANOTHER
/// workspace MUST be 404 with NO mutation — a W1 caller must not be able
/// to soft-delete (or even learn of) a W2 resource. `soft_delete` is id-
/// keyed (not workspace-scoped), so the handler is the isolation
/// boundary; the capture must stay `None`.
#[tokio::test]
async fn delete_resource_cross_workspace_is_404_no_mutation() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();

    let other_workspace = vec![0xCD_u8; 16];
    assert_ne!(
        other_workspace,
        test_ws_bytes(),
        "the cross-workspace fixture must differ from the caller's workspace"
    );
    let foreign = get_entry(id, other_workspace, "Foreign Pool", "http_pool", 8, false);

    let repo = Arc::new(FakeResourceRepo::with_get(Some(foreign)));
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = delete_resource_request(app, &id.to_string()).await;
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
        StatusCode::NOT_FOUND,
        "a resource in another workspace must be 404 on delete (no \
         cross-tenant delete; no existence leak) — got {status}"
    );
    assert!(
        repo.soft_deleted().is_none(),
        "the foreign-workspace row must NEVER be soft-deleted — \
         isolation must short-circuit before any mutation"
    );
    assert!(
        !raw.contains("Foreign Pool"),
        "cross-workspace 404 must not echo the foreign row; body: {raw}"
    );
}

/// Deleting an id with no backing row is an indistinguishable 404, no
/// mutation.
#[tokio::test]
async fn delete_resource_unknown_id_is_404_no_mutation() {
    let api_config = ApiConfig::for_test();
    let repo = Arc::new(FakeResourceRepo::with_get(None));
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = delete_resource_request(app, &ResourceId::new().to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an id with no backing row must be 404 on delete"
    );
    assert!(
        repo.soft_deleted().is_none(),
        "an absent target must not reach soft_delete"
    );
}

/// Deleting an already soft-deleted row is 404 (a tombstone is not a
/// resource) — idempotent-by-absence, no re-mutation.
#[tokio::test]
async fn delete_resource_already_deleted_is_404() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let tombstone = get_entry(id, test_ws_bytes(), "Deleted Pool", "http_pool", 7, true);

    let repo = Arc::new(FakeResourceRepo::with_get(Some(tombstone)));
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = delete_resource_request(app, &id.to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an already soft-deleted resource is a tombstone → 404"
    );
    assert!(
        repo.soft_deleted().is_none(),
        "an already-tombstoned row must not be soft-deleted again"
    );
}

/// An unparsable id is 404 on delete (indistinguishable), never 400/500.
#[tokio::test]
async fn delete_resource_malformed_id_is_404() {
    let api_config = ApiConfig::for_test();
    let repo = Arc::new(FakeResourceRepo::with_get(None));
    let state = state_with_repo(&api_config, Arc::clone(&repo) as Arc<dyn ResourceRepo>);
    let app = app::build_app(state, &api_config);

    let response = delete_resource_request(app, "not-a-valid-resource-id").await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an unparsable resource id must be 404 on delete, never 500"
    );
}

/// No resource repo configured ⇒ 503 on delete (same convention).
#[tokio::test]
async fn delete_resource_without_repo_is_503() {
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

    let response = delete_resource_request(app, &ResourceId::new().to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured resource repo must be 503 on delete"
    );
}

/// The CAS→409 mapping, asserted directly at the type level (no fake
/// `nebula-resource` needed): the handler maps the **specific**
/// `StorageError::Conflict` variant — not a catch-all over every
/// `StorageError` — to `ApiError::Conflict` (409). Any other
/// `StorageError` stays a 500. This pins that a stale `expected_version`
/// cannot be mis-signalled as a 500 and an unrelated storage fault cannot
/// be mis-signalled as a 409.
#[test]
fn storage_cas_conflict_maps_to_409_other_storage_errors_stay_500() {
    use nebula_api::ApiError;

    let cas = nebula_api::map_resource_update_storage_error(
        nebula_storage::StorageError::conflict("resource", "res_x", 3, 7),
    );
    let (status, _) = cas.to_problem_details();
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a CAS StorageError::Conflict must map to 409 Conflict"
    );

    // A non-CAS storage fault must NOT be coerced to 409 — it stays 500.
    let other = nebula_api::map_resource_update_storage_error(
        nebula_storage::StorageError::Connection("db down".to_owned()),
    );
    let (status, _) = other.to_problem_details();
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a non-CAS StorageError must stay 500, never be mis-mapped to 409"
    );
    // And it is the generic Storage arm (no internal detail leak).
    assert!(
        matches!(other, ApiError::Storage(_)),
        "non-CAS storage errors must remain the opaque Storage variant"
    );
}

/// End-to-end interplay: a successful soft-delete followed by a GET of
/// the same id is 404 — the get-by-id filter already excludes
/// `deleted_at.is_some()`, so a deleted resource is indistinguishable
/// from a missing one. Modelled with two repos (the fake is stateless):
/// delete succeeds (204) on a live row; a GET against a fixture whose row
/// is now a tombstone is 404.
#[tokio::test]
async fn delete_then_get_same_id_is_404() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();

    // 1) DELETE a live row → 204.
    let live = get_entry(id, test_ws_bytes(), "My Pool", "http_pool", 3, false);
    let del_repo = Arc::new(FakeResourceRepo::with_get(Some(live)));
    let del_state = state_with_repo(&api_config, Arc::clone(&del_repo) as Arc<dyn ResourceRepo>);
    let del_app = app::build_app(del_state, &api_config);
    let del_resp = delete_resource_request(del_app, &id.to_string()).await;
    assert_eq!(
        del_resp.status(),
        StatusCode::NO_CONTENT,
        "the soft-delete itself must be 204"
    );
    assert_eq!(
        del_repo.soft_deleted().as_deref(),
        Some(id.as_bytes().as_slice()),
        "soft_delete must have been issued for that id"
    );

    // 2) Post-delete the row is a tombstone; GET of the same id → 404
    //    (the get-by-id filter excludes `deleted_at.is_some()`).
    let tombstone = get_entry(id, test_ws_bytes(), "My Pool", "http_pool", 3, true);
    let get_app = app::build_app(get_state(&api_config, Some(tombstone)), &api_config);
    let get_resp = get_resource_request(get_app, &id.to_string()).await;
    assert_eq!(
        get_resp.status(),
        StatusCode::NOT_FOUND,
        "a GET after soft-delete must be 404 — a deleted resource is \
         indistinguishable from a missing one"
    );
}

// ── get_resource_status (read-only runtime projection) ───────────────────────
//
// `GET .../resources/{res}/status` is a READ-ONLY runtime-status
// projection. Resource lifecycle (acquire/release/drain/reload) is owned
// by the engine and is NOT exposed over HTTP (INTEGRATION_MODEL §13.1):
// there is no `{res}/acquire` (or release/drain) route at all, asserted
// below as the explicit non-lifecycle guarantee.
//
// Tenant isolation composes with the read path: the handler first
// establishes config-row ownership through the SAME audited
// `fetch_owned_resource` boundary (foreign / missing / soft-deleted /
// unparsable ⇒ an indistinguishable 404, no status oracle), and only
// then projects the engine status seam for the confirmed-owned resource.
// The seam is api-safe (`nebula_engine::EngineResourceStatus` →
// `ResourceRuntimeStatus`), so the API crate never depends on
// `nebula-resource` (deny.toml `[[wrappers]]`). The status DTO carries
// phase/health only — never config or credential material (ADR-0028 §7).

/// Fake [`EngineResourceStatus`] for the handler tests.
///
/// The API crate cannot construct a real `nebula_resource::Manager` (no
/// `nebula-resource` dependency), so the read-only status seam is faked
/// at the api-safe boundary: `runtime_status` returns a fixed
/// [`ResourceRuntimeStatus`] for `present_key` and `None` for everything
/// else (the "configured but never activated" path). The looked-up key
/// is captured so a test can prove the handler queried the seam with the
/// confirmed-owned resource's kind, never an attacker-influenced value.
struct FakeResourceStatus {
    present_key: Option<String>,
    status: ResourceRuntimeStatus,
    last_query: Mutex<Option<String>>,
}

impl FakeResourceStatus {
    /// Seam that resolves `key` (a resource `kind`) to `status`, and
    /// `None` for any other key.
    fn resolving(key: &str, status: ResourceRuntimeStatus) -> Self {
        Self {
            present_key: Some(key.to_owned()),
            status,
            last_query: Mutex::new(None),
        }
    }

    /// Seam that has no live runtime for ANY key (every resource is
    /// "configured but not currently active").
    fn empty() -> Self {
        Self {
            present_key: None,
            status: ResourceRuntimeStatus {
                phase: "ready",
                healthy: true,
                accepting: true,
            },
            last_query: Mutex::new(None),
        }
    }

    fn queried(&self) -> Option<String> {
        self.last_query
            .lock()
            .expect("status query capture mutex not poisoned")
            .clone()
    }
}

impl EngineResourceStatus for FakeResourceStatus {
    fn runtime_status(&self, key: &ResourceKey) -> Option<ResourceRuntimeStatus> {
        *self
            .last_query
            .lock()
            .expect("status query capture mutex not poisoned") = Some(key.as_str().to_owned());
        match &self.present_key {
            Some(present) if present == key.as_str() => Some(self.status.clone()),
            _ => None,
        }
    }
}

async fn get_status_request(app: axum::Router, res_id: &str) -> axum::http::Response<Body> {
    let token = create_test_jwt();
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(ws_path(&format!("/resources/{res_id}/status")))
            .header("authorization", format!("Bearer {token}"))
            .header("x-csrf-token", TEST_CSRF_TOKEN)
            .header("cookie", TEST_CSRF_COOKIE)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

/// A live, owned resource with a live runtime ⇒ **200** + a
/// `ResourceStatusDto` projecting phase/health. The raw config must NOT
/// leak (ADR-0028 §7) and no secret/credential field may appear.
#[tokio::test]
async fn get_resource_status_owned_active_is_200_with_dto() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    // The stored row's `kind` is what the handler must use to key the
    // engine status seam.
    let row = get_entry(id, test_ws_bytes(), "HTTP Pool", "http_pool", 3, false);
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(Some(row)));
    let status = Arc::new(FakeResourceStatus::resolving(
        "http_pool",
        ResourceRuntimeStatus {
            phase: "ready",
            healthy: true,
            accepting: true,
        },
    ));
    let state = state_with_repo(&api_config, repo).with_resource_status(Arc::clone(&status) as _);
    let app = app::build_app(state, &api_config);

    let response = get_status_request(app, &id.to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a live owned resource with a live runtime must be 200"
    );

    let raw = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .expect("body is utf-8");

    // No secret / config material in a status body (ADR-0028 §7): the
    // fixture's config is deliberately secret-shaped.
    assert!(
        !raw.contains("secret_looking_key") && !raw.contains("do-not-leak"),
        "status response must not surface raw resource config; body: {raw}"
    );

    let json: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON body");
    assert_eq!(json["phase"], "ready", "phase must be the projected phase");
    assert_eq!(json["healthy"], true, "healthy must be projected");
    assert_eq!(json["accepting"], true, "accepting must be projected");
    assert_eq!(
        json["id"].as_str().expect("id is a string"),
        id.to_string(),
        "id must round-trip to the same res_<ULID>"
    );

    // The DTO must expose ONLY phase/health/id — never config/secret keys.
    let obj = json.as_object().expect("status body is a JSON object");
    let allowed = ["id", "phase", "healthy", "accepting"];
    for k in obj.keys() {
        assert!(
            allowed.contains(&k.as_str()),
            "status DTO leaked an unexpected field `{k}` (no config / \
             credential material may appear — ADR-0028 §7)"
        );
    }

    // The handler keyed the seam with the CONFIRMED row's kind.
    assert_eq!(
        status.queried().as_deref(),
        Some("http_pool"),
        "the engine status seam must be queried with the owned row's kind"
    );
}

/// SECURITY (tenant isolation): a resource owned by ANOTHER workspace
/// MUST be **404** — indistinguishable from missing — and the engine
/// status seam must NEVER be consulted (no cross-tenant status oracle).
/// `fetch_owned_resource` short-circuits before the seam.
#[tokio::test]
async fn get_resource_status_cross_workspace_is_404_no_status_oracle() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();

    let other_workspace = vec![0xAB_u8; 16];
    assert_ne!(
        other_workspace,
        test_ws_bytes(),
        "the cross-workspace fixture must differ from the caller's workspace"
    );
    let foreign = get_entry(id, other_workspace, "Foreign Pool", "http_pool", 9, false);
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(Some(foreign)));
    // The seam would resolve `http_pool` — but isolation must stop the
    // request before it is ever consulted.
    let status = Arc::new(FakeResourceStatus::resolving(
        "http_pool",
        ResourceRuntimeStatus {
            phase: "ready",
            healthy: true,
            accepting: true,
        },
    ));
    let state = state_with_repo(&api_config, repo).with_resource_status(Arc::clone(&status) as _);
    let app = app::build_app(state, &api_config);

    let response = get_status_request(app, &id.to_string()).await;
    let st = response.status();
    let raw = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .expect("body is utf-8");

    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "a resource in another workspace must be 404 (no cross-tenant \
         status; no existence leak) — got {st}, body: {raw}"
    );
    assert!(
        status.queried().is_none(),
        "the engine status seam must NOT be consulted for a \
         foreign-workspace resource — isolation short-circuits first"
    );
    assert!(
        !raw.contains("Foreign Pool")
            && !raw.contains("secret_looking_key")
            && !raw.contains("do-not-leak"),
        "cross-workspace 404 must not echo the foreign resource; body: {raw}"
    );
}

/// An unknown id is an indistinguishable 404, seam never consulted.
#[tokio::test]
async fn get_resource_status_unknown_id_is_404() {
    let api_config = ApiConfig::for_test();
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(None));
    let status = Arc::new(FakeResourceStatus::empty());
    let state = state_with_repo(&api_config, repo).with_resource_status(Arc::clone(&status) as _);
    let app = app::build_app(state, &api_config);

    let response = get_status_request(app, &ResourceId::new().to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an id with no backing row must be 404 on status"
    );
    assert!(
        status.queried().is_none(),
        "an absent target must not reach the engine status seam"
    );
}

/// An unparsable id is 404 (indistinguishable), never 400/500.
#[tokio::test]
async fn get_resource_status_malformed_id_is_404() {
    let api_config = ApiConfig::for_test();
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(None));
    let status = Arc::new(FakeResourceStatus::empty());
    let state = state_with_repo(&api_config, repo).with_resource_status(status as _);
    let app = app::build_app(state, &api_config);

    let response = get_status_request(app, "not-a-valid-resource-id").await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an unparsable resource id must be 404 on status, never 500"
    );
}

/// The resource EXISTS and is owned, but has no live runtime in the
/// engine (never acquired). That is **200** with a well-defined
/// "not currently active" status — NOT 404 (the config row exists; it is
/// merely not running).
#[tokio::test]
async fn get_resource_status_owned_but_not_active_is_200_inactive() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let row = get_entry(id, test_ws_bytes(), "Idle Pool", "http_pool", 1, false);
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(Some(row)));
    // Seam resolves NOTHING → the owned resource is configured but not
    // currently active.
    let status = Arc::new(FakeResourceStatus::empty());
    let state = state_with_repo(&api_config, repo).with_resource_status(Arc::clone(&status) as _);
    let app = app::build_app(state, &api_config);

    let response = get_status_request(app, &id.to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "an owned-but-not-activated resource exists as config → 200 with \
         an inactive status, NOT 404"
    );

    let json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .expect("valid JSON body");
    assert_eq!(
        json["phase"], "inactive",
        "a configured-but-never-activated resource must report a \
         well-defined inactive phase"
    );
    assert_eq!(
        json["healthy"], false,
        "an inactive resource is not healthy"
    );
    assert_eq!(
        json["accepting"], false,
        "an inactive resource is not accepting work"
    );
    assert_eq!(
        status.queried().as_deref(),
        Some("http_pool"),
        "the seam is still consulted for the owned resource (it just \
         has no live runtime)"
    );
}

/// No status backend configured ⇒ the honest non-faked path: **503**
/// (the absent-optional-backend convention — a missing optional backend
/// is 503, never a fabricated status). Ownership is still verified first (the row is
/// owned), so a 503 here is specifically "status backend missing", not
/// an isolation outcome.
#[tokio::test]
async fn get_resource_status_without_status_backend_is_503() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let row = get_entry(id, test_ws_bytes(), "HTTP Pool", "http_pool", 3, false);
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(Some(row)));
    // `.with_resource_status(...)` deliberately NOT called.
    let state = state_with_repo(&api_config, repo);
    let app = app::build_app(state, &api_config);

    let response = get_status_request(app, &id.to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "no status backend ⇒ 503 (honest None path), never a fabricated \
         status"
    );
}

/// No resource repo configured ⇒ 503 (same convention as the other
/// resource endpoints), checked before any isolation/status work.
#[tokio::test]
async fn get_resource_status_without_repo_is_503() {
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

    let response = get_status_request(app, &ResourceId::new().to_string()).await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured resource repo must be 503 on status"
    );
}

/// Resource lifecycle is NOT exposed over HTTP (INTEGRATION_MODEL
/// §13.1). There is deliberately **no**
/// `POST .../resources/{res}/acquire` route (nor release/drain): the only
/// `{res}/...` sub-route is the read-only `{res}/status` GET.
///
/// The guarantee under test is *route absence*: a lifecycle POST must be
/// rejected exactly like any path that names no route — it must NEVER be
/// a success and never reach a resource handler. The precise status of an
/// unmatched path is **not** decided by the resource route table: the
/// merged `/internal/v1/*` router carries a blanket auth layer that
/// becomes the whole-app fallback for unmatched paths (401 with a token
/// configured, 503 without — an artifact of the internal-route
/// subsystem, unrelated to resources and pre-existing this endpoint). A
/// brittle literal-404 assertion would therefore be testing that
/// unrelated fallback, not the no-lifecycle guarantee.
///
/// So the guarantee is proven structurally: `POST .../{res}/acquire` returns the
/// **same** response as `POST .../{res}/<a definitely-nonexistent
/// sub-segment>` (equivalence-to-nonexistent — there is no special
/// acquire handling), and that response is **not** a 2xx (no lifecycle
/// route handled it). A faked status or a lifecycle mutation over HTTP
/// would both be worse than this honest absence.
#[tokio::test]
async fn post_resource_acquire_route_does_not_exist() {
    let api_config = ApiConfig::for_test();
    let id = ResourceId::new();
    let row = get_entry(id, test_ws_bytes(), "HTTP Pool", "http_pool", 3, false);
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo::with_get(Some(row)));
    let status = Arc::new(FakeResourceStatus::resolving(
        "http_pool",
        ResourceRuntimeStatus {
            phase: "ready",
            healthy: true,
            accepting: true,
        },
    ));
    let state = state_with_repo(&api_config, repo).with_resource_status(status as _);
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    async fn lifecycle_post(app: axum::Router, token: &str, suffix: String) -> StatusCode {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&suffix))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    // The lifecycle verb the engine owns and we deliberately do NOT mount.
    let acquire = lifecycle_post(app.clone(), &token, format!("/resources/{id}/acquire")).await;
    // A control sub-segment that unambiguously names no route.
    let control = lifecycle_post(
        app.clone(),
        &token,
        format!("/resources/{id}/__definitely_no_route__"),
    )
    .await;

    // `acquire` is handled identically to a path that names no route
    // — there is no acquire/release/drain route, so it is rejected purely
    // as "no such route", never specially.
    assert_eq!(
        acquire, control,
        "a lifecycle POST must be rejected exactly like any nonexistent \
         route — there must be NO acquire/release/drain route (resource \
         lifecycle is engine-owned, INTEGRATION_MODEL §13.1)"
    );

    // And it is unambiguously NOT a success: no lifecycle mutation can
    // ever happen over HTTP.
    assert!(
        !acquire.is_success(),
        "a lifecycle POST must NEVER succeed — got {acquire}"
    );
}
