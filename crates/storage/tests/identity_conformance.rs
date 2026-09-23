//! Identity-zoo behavioral conformance matrix (spec-16 §5 / §9, §6.1).
//!
//! One backend-agnostic contract suite for the nine identity aggregates
//! (`User`, `Org`, `Workspace`, `Membership`, `Resource`, `Trigger`,
//! `Quota`, `Audit`, `Blob`), exercised across
//! `{InMemory, SQLite :memory:, Postgres (DATABASE_URL-gated)}`.
//!
//! Each backend implements [`IdentityBackend`]; the shared assertions
//! encode the abstract contract every adapter must satisfy:
//!
//! - create → get round-trip
//! - first-writer-wins uniqueness on email / slug among active rows
//! - optimistic CAS (`update` with a stale `expected_version` ⇒ `Conflict`)
//! - soft-delete hides the row from every read path
//! - tenant / parent scope isolation: a cross-scope `get` returns `None`,
//!   never another tenant's row (no existence oracle, spec §6.1)
//!
//! Skip-clean policy: the SQLite case skips when built without
//! `--features sqlite`; the Postgres case skips without `DATABASE_URL`.
//! A skipped backend prints a WARN and passes — never a false green,
//! never a hard failure on a host that cannot run that backend. Backends
//! whose identity adapter does not exist yet return their stores via
//! `unimplemented!()` behind that skip guard, so the suite compiles and
//! only the live backend's cases run.

#![expect(
    clippy::print_stderr,
    reason = "conformance harness reports skip/diagnostic lines to stderr"
)]

use std::future::Future;
use std::sync::Arc;

use nebula_storage_port::Scope;
use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome,
    OrgMembershipRole, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TenantMembershipSnapshot, TriggerRow, UserRow, WorkspaceMemberUpsert, WorkspaceMembershipRole,
    WorkspaceRow,
};
use nebula_storage_port::store::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore, TriggerStore,
    UserStore, WorkspaceStore,
};
use rstest::rstest;

/// A storage backend under identity conformance test.
#[async_trait::async_trait]
trait IdentityBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn user_store(&self) -> Arc<dyn UserStore>;
    async fn org_store(&self) -> Arc<dyn OrgStore>;
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore>;
    async fn membership_store(&self) -> Arc<dyn MembershipStore>;
    async fn resource_store(&self) -> Arc<dyn ResourceStore>;
    async fn trigger_store(&self) -> Arc<dyn TriggerStore>;
    async fn quota_store(&self) -> Arc<dyn QuotaStore>;
    async fn audit_store(&self) -> Arc<dyn AuditStore>;
    async fn blob_store(&self) -> Arc<dyn BlobStore>;
}

// ── InMemory backend (always available) ───────────────────────────────────

#[derive(Default)]
struct InMemoryBackend {
    directory: nebula_storage::inmem::InMemoryIdentityDirectory,
}

#[async_trait::async_trait]
impl IdentityBackend for InMemoryBackend {
    fn name(&self) -> &'static str {
        "InMemory"
    }
    async fn user_store(&self) -> Arc<dyn UserStore> {
        Arc::new(nebula_storage::inmem::InMemoryUserStore::new())
    }
    async fn org_store(&self) -> Arc<dyn OrgStore> {
        Arc::new(self.directory.org_store())
    }
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore> {
        Arc::new(self.directory.workspace_store())
    }
    async fn membership_store(&self) -> Arc<dyn MembershipStore> {
        Arc::new(self.directory.membership_store())
    }
    async fn resource_store(&self) -> Arc<dyn ResourceStore> {
        Arc::new(nebula_storage::inmem::InMemoryResourceStore::new())
    }
    async fn trigger_store(&self) -> Arc<dyn TriggerStore> {
        Arc::new(nebula_storage::inmem::InMemoryTriggerStore::new())
    }
    async fn quota_store(&self) -> Arc<dyn QuotaStore> {
        Arc::new(nebula_storage::inmem::InMemoryQuotaStore::new())
    }
    async fn audit_store(&self) -> Arc<dyn AuditStore> {
        Arc::new(nebula_storage::inmem::InMemoryAuditStore::new())
    }
    async fn blob_store(&self) -> Arc<dyn BlobStore> {
        Arc::new(nebula_storage::inmem::InMemoryBlobStore::new())
    }
}

// ── SQLite backend (built only with `--features sqlite`) ──────────────────

/// Each `SqliteBackend` instance owns one shared-cache in-memory database
/// (so a `create` and a later read observe the same rows), created lazily
/// on first store request. Only built when the `sqlite` feature is on;
/// without it the case skips like Postgres.
#[derive(Default)]
struct SqliteBackend {
    #[cfg(feature = "sqlite")]
    pool: tokio::sync::OnceCell<sqlx::SqlitePool>,
}

#[cfg(feature = "sqlite")]
impl SqliteBackend {
    async fn pool(&self) -> sqlx::SqlitePool {
        use std::str::FromStr;
        self.pool
            .get_or_init(|| async {
                let db_name = format!("nebula-identity-{}", uuid::Uuid::new_v4());
                let url = format!("sqlite:file:{db_name}?mode=memory&cache=shared");
                let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&url)
                    .expect("parse sqlite memory url")
                    .create_if_missing(true);
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(4)
                    .connect_with(opts)
                    .await
                    .expect("connect sqlite memory");
                nebula_storage::sqlite::init_schema(&pool)
                    .await
                    .expect("install port schema");
                pool
            })
            .await
            .clone()
    }
}

#[async_trait::async_trait]
impl IdentityBackend for SqliteBackend {
    fn name(&self) -> &'static str {
        "Sqlite(:memory:)"
    }
    async fn user_store(&self) -> Arc<dyn UserStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteUserStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn org_store(&self) -> Arc<dyn OrgStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteOrgStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteWorkspaceStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn membership_store(&self) -> Arc<dyn MembershipStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteMembershipStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn resource_store(&self) -> Arc<dyn ResourceStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteResourceStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn trigger_store(&self) -> Arc<dyn TriggerStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteTriggerStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn quota_store(&self) -> Arc<dyn QuotaStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteQuotaStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn audit_store(&self) -> Arc<dyn AuditStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteAuditStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
    async fn blob_store(&self) -> Arc<dyn BlobStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteBlobStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "sqlite"))]
        unimplemented!("built without the `sqlite` feature")
    }
}

// ── Postgres backend (DATABASE_URL-gated) ─────────────────────────────────

/// Each `PostgresBackend` instance owns one pool created lazily on first
/// store request (port schema installed once). Only exercised when
/// `DATABASE_URL` is set and the crate is built with `--features
/// postgres`; otherwise the case skips cleanly.
///
/// The pool is pinned to a private schema, matching the fresh-store contract
/// the InMemory and SQLite backends give every case — see
/// `tests/support/postgres_schema.rs` for why a shared `public` schema
/// silently invalidated the Postgres arm.
#[derive(Default)]
struct PostgresBackend {
    #[cfg(feature = "postgres")]
    pool: tokio::sync::OnceCell<sqlx::PgPool>,
}

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
impl PostgresBackend {
    async fn pool(&self) -> sqlx::PgPool {
        self.pool
            .get_or_init(|| async {
                let url = std::env::var("DATABASE_URL")
                    .unwrap_or_else(|e| panic!("DATABASE_URL required for the Postgres case: {e}"));
                let pool = postgres_schema::connect_with_private_schema(
                    &url,
                    "nebula_identity_conformance",
                )
                .await
                .expect("connect Postgres (DATABASE_URL)");
                nebula_storage::postgres::init_schema(&pool)
                    .await
                    .expect("install port schema");
                pool
            })
            .await
            .clone()
    }
}

#[async_trait::async_trait]
impl IdentityBackend for PostgresBackend {
    fn name(&self) -> &'static str {
        "Postgres"
    }
    async fn user_store(&self) -> Arc<dyn UserStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgUserStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn org_store(&self) -> Arc<dyn OrgStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgOrgStore::new(self.pool().await))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgWorkspaceStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn membership_store(&self) -> Arc<dyn MembershipStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgMembershipStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn resource_store(&self) -> Arc<dyn ResourceStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgResourceStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn trigger_store(&self) -> Arc<dyn TriggerStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgTriggerStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn quota_store(&self) -> Arc<dyn QuotaStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgQuotaStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn audit_store(&self) -> Arc<dyn AuditStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgAuditStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
    async fn blob_store(&self) -> Arc<dyn BlobStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgBlobStore::new(
                self.pool().await,
            ))
        }
        #[cfg(not(feature = "postgres"))]
        unimplemented!("built without the `postgres` feature")
    }
}

fn sqlite_skip() -> Option<&'static str> {
    if cfg!(feature = "sqlite") {
        None
    } else {
        Some("SQLite identity case skipped — built without `--features sqlite`")
    }
}

fn postgres_skip() -> Option<&'static str> {
    match std::env::var("DATABASE_URL") {
        Ok(v) if !v.trim().is_empty() => None,
        _ => Some("Postgres identity case skipped — DATABASE_URL unset"),
    }
}

fn skip_reason(backend: &dyn IdentityBackend) -> Option<&'static str> {
    match backend.name() {
        "Postgres" => postgres_skip(),
        "Sqlite(:memory:)" => sqlite_skip(),
        _ => None,
    }
}

async fn run<F, Fut>(backend: Box<dyn IdentityBackend>, body: F)
where
    F: FnOnce(Box<dyn IdentityBackend>) -> Fut,
    Fut: Future<Output = ()>,
{
    if let Some(reason) = skip_reason(backend.as_ref()) {
        eprintln!("WARN [identity-conformance] {reason}");
        return;
    }
    body(backend).await;
}

fn in_memory() -> Box<dyn IdentityBackend> {
    Box::new(InMemoryBackend::default())
}

fn sqlite() -> Box<dyn IdentityBackend> {
    Box::new(SqliteBackend::default())
}

fn postgres() -> Box<dyn IdentityBackend> {
    Box::new(PostgresBackend::default())
}

// ── row builders ──────────────────────────────────────────────────────────

fn user_row(id: &str, email: &str) -> UserRow {
    UserRow {
        id: id.into(),
        email: email.into(),
        email_verified_at: None,
        display_name: "Test User".into(),
        avatar_url: None,
        password_hash: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        last_login_at: None,
        locked_until: None,
        failed_login_count: 0,
        mfa_enabled: false,
        mfa_secret_envelope: None,
        version: 0,
        deleted_at: None,
    }
}

fn org_row(id: &str, slug: &str) -> OrgRow {
    OrgRow {
        id: id.into(),
        slug: slug.into(),
        display_name: "Test Org".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        created_by: "usr_1".into(),
        plan: "free".into(),
        billing_email: None,
        settings: serde_json::json!({}),
        version: 0,
        deleted_at: None,
    }
}

fn workspace_row(id: &str, org_id: &str, slug: &str) -> WorkspaceRow {
    WorkspaceRow {
        id: id.into(),
        org_id: org_id.into(),
        slug: slug.into(),
        display_name: "Test Workspace".into(),
        description: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        created_by: "usr_1".into(),
        is_default: false,
        settings: serde_json::json!({}),
        version: 0,
        deleted_at: None,
    }
}

fn org_member(org_id: &str, principal_id: &str, role: OrgMembershipRole) -> OrgMemberUpsert {
    OrgMemberUpsert {
        org_id: org_id.into(),
        principal_kind: PrincipalKind::User,
        principal_id: principal_id.into(),
        role,
        added_by: None,
    }
}

fn workspace_member(org_id: &str, workspace_id: &str, principal_id: &str) -> WorkspaceMemberUpsert {
    WorkspaceMemberUpsert {
        org_id: org_id.into(),
        workspace_id: workspace_id.into(),
        principal_kind: PrincipalKind::User,
        principal_id: principal_id.into(),
        role: WorkspaceMembershipRole::Editor,
        added_by: None,
    }
}

fn resource_row(id: &str, workspace_id: &str, slug: &str) -> ResourceRow {
    let credential_bindings =
        std::collections::BTreeMap::from([("auth".to_owned(), "cred_test".to_owned())]);
    ResourceRow {
        id: id.into(),
        workspace_id: workspace_id.into(),
        slug: slug.into(),
        display_name: "Test Resource".into(),
        kind: "http".into(),
        config: serde_json::json!({}),
        credential_bindings,
        created_at: "2026-01-01T00:00:00Z".into(),
        created_by: "usr_1".into(),
        version: 0,
        deleted_at: None,
    }
}

fn trigger_row(id: &str, workspace_id: &str, slug: &str) -> TriggerRow {
    TriggerRow {
        id: id.into(),
        workspace_id: workspace_id.into(),
        workflow_id: "wf_1".into(),
        slug: slug.into(),
        display_name: "Test Trigger".into(),
        kind: "manual".into(),
        config: serde_json::json!({}),
        state: "active".into(),
        run_as: None,
        webhook_path: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        created_by: "usr_1".into(),
        version: 0,
        deleted_at: None,
    }
}

fn quota_row(org_id: &str, concurrent: i32) -> QuotaRow {
    QuotaRow {
        org_id: org_id.into(),
        plan: "free".into(),
        concurrent_executions_limit: 10,
        executions_per_month_limit: None,
        active_workflows_limit: None,
        concurrent_executions: concurrent,
        executions_this_month: 0,
        month_reset_at: "2026-02-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn audit_row(id: &str, org_id: &str, emitted_at: &str) -> AuditLogRow {
    AuditLogRow {
        id: id.into(),
        org_id: org_id.into(),
        workspace_id: None,
        actor_kind: "system".into(),
        actor_id: None,
        action: "workflow.created".into(),
        target_kind: None,
        target_id: None,
        details: None,
        ip_address: None,
        user_agent: None,
        emitted_at: emitted_at.into(),
    }
}

fn blob_row(id: &str, workspace_id: &str, expires_at: Option<&str>) -> BlobRow {
    BlobRow {
        id: id.into(),
        workspace_id: workspace_id.into(),
        execution_id: None,
        kind: "attachment".into(),
        content_type: None,
        size_bytes: 3,
        checksum: None,
        storage_mode: "db".into(),
        data: Some(vec![1, 2, 3]),
        external_ref: None,
        metadata: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: expires_at.map(ToString::to_string),
    }
}

// ── shared contract assertions ────────────────────────────────────────────

async fn assert_user_contract(b: &dyn IdentityBackend) {
    let s = b.user_store().await;
    s.create(user_row("usr_1", "a@example.com"))
        .await
        .expect("create user");
    // duplicate id ⇒ Duplicate
    assert!(s.create(user_row("usr_1", "z@example.com")).await.is_err());
    // duplicate active email (case-insensitive) ⇒ Duplicate
    assert!(s.create(user_row("usr_2", "A@EXAMPLE.COM")).await.is_err());
    // round-trip + email lookup
    assert_eq!(
        s.get("usr_1").await.expect("get").unwrap().email,
        "a@example.com"
    );
    assert_eq!(
        s.get_by_email("A@example.com")
            .await
            .expect("get_by_email")
            .unwrap()
            .id,
        "usr_1"
    );
    // CAS conflict
    assert!(
        s.update(user_row("usr_1", "a@example.com"), 99)
            .await
            .is_err()
    );
    let mut updated = user_row("usr_1", "a@example.com");
    updated.display_name = "Renamed".into();
    updated.version = 1;
    s.update(updated, 0).await.expect("CAS update");
    assert_eq!(
        s.get("usr_1").await.unwrap().unwrap().display_name,
        "Renamed"
    );
    // soft-delete hides the row and frees the email
    s.soft_delete("usr_1").await.expect("soft_delete");
    assert!(s.get("usr_1").await.unwrap().is_none());
    assert!(s.get_by_email("a@example.com").await.unwrap().is_none());
    s.create(user_row("usr_3", "a@example.com"))
        .await
        .expect("email freed after soft-delete");
}

async fn assert_org_contract(b: &dyn IdentityBackend) {
    let s = b.org_store().await;
    s.create(org_row("org_1", "acme"))
        .await
        .expect("create org");
    assert!(s.create(org_row("org_2", "acme")).await.is_err());
    assert_eq!(s.get_by_slug("acme").await.unwrap().unwrap().id, "org_1");
    assert!(s.update(org_row("org_1", "acme"), 7).await.is_err());
    s.soft_delete("org_1").await.expect("soft_delete");
    assert!(s.get("org_1").await.unwrap().is_none());
    assert!(s.get_by_slug("acme").await.unwrap().is_none());
}

async fn assert_workspace_contract(b: &dyn IdentityBackend) {
    let s = b.workspace_store().await;
    s.create(workspace_row("ws_1", "org_1", "main"))
        .await
        .expect("create ws");
    // same slug, different org ⇒ allowed
    s.create(workspace_row("ws_2", "org_2", "main"))
        .await
        .expect("slug unique per org");
    // duplicate slug within org ⇒ Duplicate
    assert!(
        s.create(workspace_row("ws_3", "org_1", "main"))
            .await
            .is_err()
    );
    // cross-org get is a miss (no existence oracle)
    assert!(s.get("org_2", "ws_1").await.unwrap().is_none());
    assert_eq!(s.list_for_org("org_1").await.unwrap().len(), 1);
    s.soft_delete("org_1", "ws_1").await.expect("soft_delete");
    assert!(s.get("org_1", "ws_1").await.unwrap().is_none());
    assert_eq!(s.list_for_org("org_1").await.unwrap().len(), 0);
}

async fn assert_membership_contract(b: &dyn IdentityBackend) {
    let orgs = b.org_store().await;
    orgs.create(org_row("org_1", "one")).await.unwrap();
    let s = b.membership_store().await;
    assert_eq!(
        s.upsert_org_member_guarded(org_member("org_1", "usr_1", OrgMembershipRole::Admin))
            .await
            .unwrap(),
        OrgMemberUpsertOutcome::Applied
    );
    assert_eq!(
        s.upsert_org_member_guarded(org_member("org_1", "usr_1", OrgMembershipRole::Owner))
            .await
            .unwrap(),
        OrgMemberUpsertOutcome::Applied
    );
    let got = s
        .get(ScopeKind::Org, "org_1", PrincipalKind::User, "usr_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.role, "OrgOwner");
    assert!(chrono::DateTime::parse_from_rfc3339(&got.added_at).is_ok());
    assert_eq!(
        s.list_for_scope(ScopeKind::Org, "org_1")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        s.get(ScopeKind::Org, "org_2", PrincipalKind::User, "usr_1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        s.remove_org_member_guarded("org_1", PrincipalKind::User, "usr_1")
            .await
            .unwrap(),
        OrgMemberRemoveOutcome::WouldLockOut
    );
    assert_eq!(
        s.upsert_org_member_guarded(org_member("org_1", "usr_2", OrgMembershipRole::Admin))
            .await
            .unwrap(),
        OrgMemberUpsertOutcome::Applied
    );
    assert_eq!(
        s.remove_org_member_guarded("org_1", PrincipalKind::User, "usr_1")
            .await
            .unwrap(),
        OrgMemberRemoveOutcome::Removed
    );
    assert!(
        s.get(ScopeKind::Org, "org_1", PrincipalKind::User, "usr_1")
            .await
            .unwrap()
            .is_none()
    );
}

async fn assert_membership_snapshot(b: &dyn IdentityBackend) {
    let orgs = b.org_store().await;
    let workspaces = b.workspace_store().await;
    let s = b.membership_store().await;
    orgs.create(org_row("org_a", "a")).await.unwrap();
    orgs.create(org_row("org_b", "b")).await.unwrap();
    workspaces
        .create(workspace_row("ws_a", "org_a", "a"))
        .await
        .unwrap();
    s.upsert_org_member_guarded(org_member("org_a", "same", OrgMembershipRole::Owner))
        .await
        .unwrap();
    let mut service_account = org_member("org_b", "same", OrgMembershipRole::Admin);
    service_account.principal_kind = PrincipalKind::ServiceAccount;
    s.upsert_org_member_guarded(service_account).await.unwrap();
    s.upsert_workspace_member(workspace_member("org_a", "ws_a", "same"))
        .await
        .unwrap();
    let snapshot = s
        .get_tenant_membership("org_a", Some("ws_a"), PrincipalKind::User, "same")
        .await
        .unwrap();
    assert_eq!(
        snapshot,
        TenantMembershipSnapshot {
            org_role: Some(OrgMembershipRole::Owner),
            workspace_role: Some(WorkspaceMembershipRole::Editor)
        }
    );
    assert_eq!(
        s.get_tenant_membership("org_a", None, PrincipalKind::User, "same")
            .await
            .unwrap()
            .workspace_role,
        None
    );
    assert_eq!(
        s.get_tenant_membership("org_a", Some("ws_a"), PrincipalKind::ServiceAccount, "same")
            .await
            .unwrap(),
        TenantMembershipSnapshot::default()
    );
    assert_eq!(
        s.get_tenant_membership("org_b", Some("ws_a"), PrincipalKind::User, "same")
            .await
            .unwrap(),
        TenantMembershipSnapshot::default()
    );
    let user_orgs = s
        .list_orgs_for_principal(PrincipalKind::User, "same")
        .await
        .unwrap();
    assert_eq!(user_orgs.len(), 1);
    assert_eq!(user_orgs[0].org_id, "org_a");
    assert_eq!(user_orgs[0].role, OrgMembershipRole::Owner);
    let service_orgs = s
        .list_orgs_for_principal(PrincipalKind::ServiceAccount, "same")
        .await
        .unwrap();
    assert_eq!(service_orgs.len(), 1);
    assert_eq!(service_orgs[0].org_id, "org_b");
    assert!(
        s.list_orgs_for_principal(PrincipalKind::User, "absent")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        s.upsert_workspace_member(workspace_member("org_b", "ws_a", "same"))
            .await,
        Err(nebula_storage_port::StorageError::NotFound { .. })
    ));
    assert!(
        !s.remove_workspace_member("org_b", "ws_a", PrincipalKind::User, "same")
            .await
            .unwrap()
    );
    assert_eq!(
        s.get_tenant_membership("org_a", Some("missing"), PrincipalKind::User, "same")
            .await
            .unwrap()
            .workspace_role,
        None
    );
    workspaces.soft_delete("org_a", "ws_a").await.unwrap();
    assert_eq!(
        s.get_tenant_membership("org_a", Some("ws_a"), PrincipalKind::User, "same")
            .await
            .unwrap()
            .workspace_role,
        None
    );
    assert!(matches!(
        s.upsert_workspace_member(workspace_member("org_a", "ws_a", "same"))
            .await,
        Err(nebula_storage_port::StorageError::NotFound { .. })
    ));
}

async fn assert_membership_live_and_deleted_workspace_aliases(b: &dyn IdentityBackend) {
    let orgs = b.org_store().await;
    let workspaces = b.workspace_store().await;
    let store = b.membership_store().await;
    orgs.create(org_row("org_a", "a")).await.unwrap();
    orgs.create(org_row("org_b", "b")).await.unwrap();
    workspaces
        .create(workspace_row("shared", "org_a", "a"))
        .await
        .unwrap();
    store
        .upsert_workspace_member(workspace_member("org_a", "shared", "user"))
        .await
        .unwrap();
    workspaces
        .create(workspace_row("shared", "org_b", "b"))
        .await
        .unwrap();
    for deleted_alias in [false, true] {
        if deleted_alias {
            workspaces.soft_delete("org_a", "shared").await.unwrap();
        }
        for org_id in ["org_a", "org_b"] {
            assert_eq!(
                store
                    .get_tenant_membership(org_id, Some("shared"), PrincipalKind::User, "user")
                    .await
                    .unwrap()
                    .workspace_role,
                None
            );
            assert!(matches!(
                store
                    .upsert_workspace_member(workspace_member(org_id, "shared", "user"))
                    .await,
                Err(nebula_storage_port::StorageError::NotFound { .. })
            ));
            assert!(
                !store
                    .remove_workspace_member(org_id, "shared", PrincipalKind::User, "user")
                    .await
                    .unwrap()
            );
        }
        // Rejection must leave the historical grant intact for explicit repair.
        assert_eq!(
            store
                .get(ScopeKind::Workspace, "shared", PrincipalKind::User, "user")
                .await
                .unwrap()
                .unwrap()
                .role,
            "WorkspaceEditor"
        );
    }
}

async fn assert_membership_lockout(b: &dyn IdentityBackend) {
    let orgs = b.org_store().await;
    orgs.create(org_row("org_lock", "lock")).await.unwrap();
    let s = b.membership_store().await;
    assert_eq!(
        s.upsert_org_member_guarded(org_member("org_lock", "first", OrgMembershipRole::Member))
            .await
            .unwrap(),
        OrgMemberUpsertOutcome::WouldLockOut
    );
    assert!(
        s.list_for_scope(ScopeKind::Org, "org_lock")
            .await
            .unwrap()
            .is_empty()
    );
    for role in [OrgMembershipRole::Owner, OrgMembershipRole::Admin] {
        assert_eq!(
            s.upsert_org_member_guarded(org_member("org_lock", "first", role))
                .await
                .unwrap(),
            OrgMemberUpsertOutcome::Applied
        );
        assert_eq!(
            s.upsert_org_member_guarded(org_member(
                "org_lock",
                "first",
                OrgMembershipRole::Billing
            ))
            .await
            .unwrap(),
            OrgMemberUpsertOutcome::WouldLockOut
        );
        assert_eq!(
            s.remove_org_member_guarded("org_lock", PrincipalKind::User, "first")
                .await
                .unwrap(),
            OrgMemberRemoveOutcome::WouldLockOut
        );
    }
    assert_eq!(
        s.remove_org_member_guarded("org_lock", PrincipalKind::User, "missing")
            .await
            .unwrap(),
        OrgMemberRemoveOutcome::NotFound
    );
    assert_eq!(
        s.upsert_org_member_guarded(org_member(
            "org_lock",
            "ordinary",
            OrgMembershipRole::Member
        ))
        .await
        .unwrap(),
        OrgMemberUpsertOutcome::Applied
    );
    assert_eq!(
        s.upsert_org_member_guarded(org_member(
            "org_lock",
            "ordinary",
            OrgMembershipRole::Billing
        ))
        .await
        .unwrap(),
        OrgMemberUpsertOutcome::Applied
    );
    assert_eq!(
        s.remove_org_member_guarded("org_lock", PrincipalKind::User, "ordinary")
            .await
            .unwrap(),
        OrgMemberRemoveOutcome::Removed
    );
    s.upsert_org_member_guarded(org_member("org_lock", "second", OrgMembershipRole::Owner))
        .await
        .unwrap();
    let (remove, demote) = tokio::join!(
        s.remove_org_member_guarded("org_lock", PrincipalKind::User, "first"),
        s.upsert_org_member_guarded(org_member("org_lock", "second", OrgMembershipRole::Member))
    );
    assert!(matches!(
        (remove.unwrap(), demote.unwrap()),
        (
            OrgMemberRemoveOutcome::Removed,
            OrgMemberUpsertOutcome::WouldLockOut
        ) | (
            OrgMemberRemoveOutcome::WouldLockOut,
            OrgMemberUpsertOutcome::Applied
        )
    ));
    let rows = s.list_for_scope(ScopeKind::Org, "org_lock").await.unwrap();
    assert_eq!(
        rows.iter()
            .filter(|row| OrgMembershipRole::parse(&row.role).unwrap().is_privileged())
            .count(),
        1
    );
}

async fn assert_resource_contract(b: &dyn IdentityBackend) {
    let s = b.resource_store().await;
    let a = Scope::new("ws_a", "org_a");
    let other = Scope::new("ws_b", "org_b");
    s.create(&a, resource_row("res_1", "ws_a", "db"))
        .await
        .expect("create");
    assert!(
        s.create(&a, resource_row("res_2", "ws_a", "db"))
            .await
            .is_err()
    );
    // cross-scope get is a miss
    assert!(s.get(&other, "res_1").await.unwrap().is_none());
    let listed = s.list(&a).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0]
            .credential_bindings
            .get("auth")
            .map(String::as_str),
        Some("cred_test"),
        "credential bindings must round-trip separately from resource config"
    );
    assert!(
        s.update(&a, resource_row("res_1", "ws_a", "db"), 42)
            .await
            .is_err()
    );
    s.soft_delete(&a, "res_1").await.expect("soft_delete");
    assert!(s.get(&a, "res_1").await.unwrap().is_none());
    assert_eq!(s.list(&a).await.unwrap().len(), 0);
}

async fn assert_trigger_contract(b: &dyn IdentityBackend) {
    let s = b.trigger_store().await;
    let a = Scope::new("ws_a", "org_a");
    let other = Scope::new("ws_b", "org_b");
    s.create(&a, trigger_row("trg_1", "ws_a", "cron"))
        .await
        .expect("create");
    assert!(s.get(&other, "trg_1").await.unwrap().is_none());
    assert_eq!(s.list(&a).await.unwrap().len(), 1);
    assert!(
        s.update(&a, trigger_row("trg_1", "ws_a", "cron"), 5)
            .await
            .is_err()
    );
    s.soft_delete(&a, "trg_1").await.expect("soft_delete");
    assert!(s.get(&a, "trg_1").await.unwrap().is_none());
}

async fn assert_quota_contract(b: &dyn IdentityBackend) {
    let s = b.quota_store().await;
    s.upsert(quota_row("org_1", 0)).await.expect("upsert");
    assert_eq!(
        s.get("org_1").await.unwrap().unwrap().concurrent_executions,
        0
    );
    assert_eq!(s.adjust_concurrent("org_1", 3).await.expect("adjust"), 3);
    assert_eq!(s.adjust_concurrent("org_1", -1).await.expect("adjust"), 2);
    // cannot go below zero
    assert!(s.adjust_concurrent("org_1", -10).await.is_err());
    assert_eq!(
        s.get("org_1").await.unwrap().unwrap().concurrent_executions,
        2
    );
    // missing org ⇒ NotFound
    assert!(s.adjust_concurrent("org_missing", 1).await.is_err());
}

async fn assert_audit_contract(b: &dyn IdentityBackend) {
    let s = b.audit_store().await;
    s.append(audit_row("aud_1", "org_1", "2026-01-01T00:00:00Z"))
        .await
        .expect("append");
    s.append(audit_row("aud_2", "org_1", "2026-01-02T00:00:00Z"))
        .await
        .expect("append");
    s.append(audit_row("aud_x", "org_2", "2026-01-03T00:00:00Z"))
        .await
        .expect("append");
    let rows = s.list_for_org("org_1", 10).await.expect("list");
    assert_eq!(rows.len(), 2, "org-scoped");
    // newest first
    assert_eq!(rows[0].id, "aud_2");
    assert_eq!(rows[1].id, "aud_1");
    // limit honoured
    assert_eq!(s.list_for_org("org_1", 1).await.unwrap().len(), 1);
}

async fn assert_blob_contract(b: &dyn IdentityBackend) {
    let s = b.blob_store().await;
    s.put(blob_row("blb_1", "ws_a", None)).await.expect("put");
    s.put(blob_row("blb_2", "ws_a", Some("2000-01-01T00:00:00Z")))
        .await
        .expect("put expiring");
    assert_eq!(s.get("ws_a", "blb_1").await.unwrap().unwrap().size_bytes, 3);
    // cross-workspace get is a miss
    assert!(s.get("ws_b", "blb_1").await.unwrap().is_none());
    // evict_expired removes the past-expiry blob only
    assert_eq!(s.evict_expired().await.expect("evict"), 1);
    assert!(s.get("ws_a", "blb_2").await.unwrap().is_none());
    assert!(s.get("ws_a", "blb_1").await.unwrap().is_some());
    s.delete("ws_a", "blb_1").await.expect("delete");
    assert!(s.get("ws_a", "blb_1").await.unwrap().is_none());
}

// ── matrix ────────────────────────────────────────────────────────────────

macro_rules! identity_matrix {
    ($name:ident, $assertion:path) => {
        #[rstest]
        #[case::in_memory(in_memory())]
        #[case::sqlite(sqlite())]
        #[case::postgres(postgres())]
        #[tokio::test]
        async fn $name(#[case] backend: Box<dyn IdentityBackend>) {
            run(backend, |b| async move { $assertion(b.as_ref()).await }).await;
        }
    };
}

identity_matrix!(user_store_contract, assert_user_contract);
identity_matrix!(org_store_contract, assert_org_contract);
identity_matrix!(workspace_store_contract, assert_workspace_contract);
identity_matrix!(membership_store_contract, assert_membership_contract);
identity_matrix!(membership_snapshot, assert_membership_snapshot);
identity_matrix!(
    membership_live_and_deleted_workspace_aliases,
    assert_membership_live_and_deleted_workspace_aliases
);
identity_matrix!(membership_lockout, assert_membership_lockout);
identity_matrix!(resource_store_contract, assert_resource_contract);
identity_matrix!(trigger_store_contract, assert_trigger_contract);
identity_matrix!(quota_store_contract, assert_quota_contract);
identity_matrix!(audit_store_contract, assert_audit_contract);
identity_matrix!(blob_store_contract, assert_blob_contract);

/// File SQLite exercises real competing connections rather than relying only
/// on shared-cache memory's lock behavior.
#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn membership_lockout_file_sqlite() {
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("membership.db"))
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let backend = SqliteBackend::default();
    backend.pool.set(pool.clone()).unwrap();
    assert_membership_lockout(&backend).await;
    pool.close().await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn membership_corrupt_roles_fail_closed_sqlite() {
    let backend = SqliteBackend::default();
    let pool = backend.pool().await;
    let orgs = backend.org_store().await;
    let workspaces = backend.workspace_store().await;
    let store = backend.membership_store().await;
    orgs.create(org_row("org", "org")).await.unwrap();
    store
        .upsert_org_member_guarded(org_member("org", "owner", OrgMembershipRole::Owner))
        .await
        .unwrap();
    for role in ["unknown", "WorkspaceAdmin"] {
        sqlx::query("UPDATE port_memberships SET role = ? WHERE scope_kind = 'org'")
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            store
                .get_tenant_membership("org", None, PrincipalKind::User, "owner")
                .await,
            Err(nebula_storage_port::StorageError::Serialization(_))
        ));
        assert!(matches!(
            store
                .list_orgs_for_principal(PrincipalKind::User, "owner")
                .await,
            Err(nebula_storage_port::StorageError::Serialization(_))
        ));
        assert!(matches!(
            store
                .upsert_org_member_guarded(org_member(
                    "org",
                    "replacement",
                    OrgMembershipRole::Admin
                ))
                .await,
            Err(nebula_storage_port::StorageError::Serialization(_))
        ));
        assert!(matches!(
            store
                .remove_org_member_guarded("org", PrincipalKind::User, "owner")
                .await,
            Err(nebula_storage_port::StorageError::Serialization(_))
        ));
        assert_eq!(
            store
                .list_for_scope(ScopeKind::Org, "org")
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .get(ScopeKind::Org, "org", PrincipalKind::User, "owner")
                .await
                .unwrap()
                .unwrap()
                .role,
            role
        );
    }
    sqlx::query("UPDATE port_memberships SET role = 'OrgOwner' WHERE scope_kind = 'org'")
        .execute(&pool)
        .await
        .unwrap();
    workspaces
        .create(workspace_row("ws", "org", "ws"))
        .await
        .unwrap();
    store
        .upsert_workspace_member(workspace_member("org", "ws", "owner"))
        .await
        .unwrap();
    sqlx::query("UPDATE port_memberships SET role = 'OrgOwner' WHERE scope_kind = 'workspace'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        store
            .get_tenant_membership("org", Some("ws"), PrincipalKind::User, "owner")
            .await,
        Err(nebula_storage_port::StorageError::Serialization(_))
    ));
}
