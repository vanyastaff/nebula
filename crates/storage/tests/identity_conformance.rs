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

use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome,
    OrgMembershipRole, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TenantDefaultWorkspaceCreate, TenantMembershipSnapshot, TenantOrgCreate,
    TenantProvisioningConflict, TenantProvisioningOutcome, TenantProvisioningRequest, TriggerRow,
    UserRow, WorkspaceMemberUpsert, WorkspaceMembershipRole, WorkspaceRow,
};
use nebula_storage_port::store::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore,
    TenantProvisioningStore, TriggerStore, UserStore, WorkspaceStore,
};
use nebula_storage_port::{Scope, StorageError as PortStorageError};
use rstest::rstest;

/// A storage backend under identity conformance test.
#[async_trait::async_trait]
trait IdentityBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn user_store(&self) -> Arc<dyn UserStore>;
    async fn org_store(&self) -> Arc<dyn OrgStore>;
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore>;
    async fn membership_store(&self) -> Arc<dyn MembershipStore>;
    async fn tenant_provisioning_store(&self) -> Arc<dyn TenantProvisioningStore>;
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
    async fn tenant_provisioning_store(&self) -> Arc<dyn TenantProvisioningStore> {
        Arc::new(self.directory.provisioning_store())
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
    async fn tenant_provisioning_store(&self) -> Arc<dyn TenantProvisioningStore> {
        #[cfg(feature = "sqlite")]
        {
            Arc::new(nebula_storage::sqlite::SqliteTenantProvisioningStore::new(
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
    async fn tenant_provisioning_store(&self) -> Arc<dyn TenantProvisioningStore> {
        #[cfg(feature = "postgres")]
        {
            Arc::new(nebula_storage::postgres::PgTenantProvisioningStore::new(
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

fn tenant_request(org_id: &str, org_slug: &str, workspace_id: &str) -> TenantProvisioningRequest {
    let org = TenantOrgCreate::new(
        org_id.into(),
        org_slug.into(),
        "Test Org".into(),
        "usr_1".into(),
        "free".into(),
        None,
        serde_json::json!({}),
    )
    .unwrap();
    let workspace = TenantDefaultWorkspaceCreate::new(
        workspace_id.into(),
        "default".into(),
        "Test Workspace".into(),
        None,
        "usr_1".into(),
        serde_json::json!({}),
    )
    .unwrap();
    TenantProvisioningRequest::new(
        org,
        workspace,
        PrincipalKind::User,
        "owner".into(),
        Some("bootstrap".into()),
    )
    .unwrap()
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
    assert_eq!(
        s.get_by_slug("org_1", "main").await.unwrap().unwrap().id,
        "ws_1"
    );
    assert_eq!(
        s.get_by_slug("org_2", "main").await.unwrap().unwrap().id,
        "ws_2"
    );
    assert!(s.get_by_slug("org_2", "missing").await.unwrap().is_none());
    assert_eq!(s.list_for_org("org_1").await.unwrap().len(), 1);

    let mut default = workspace_row("ws_default", "org_1", "default");
    default.is_default = true;
    s.create(default).await.expect("create default workspace");
    let mut second_default = workspace_row("ws_other_default", "org_1", "other-default");
    second_default.is_default = true;
    assert!(matches!(
        s.create(second_default).await,
        Err(PortStorageError::Duplicate { .. })
    ));
    let mut promote = workspace_row("ws_1", "org_1", "main");
    promote.is_default = true;
    assert!(matches!(
        s.update(promote.clone(), 7).await,
        Err(PortStorageError::Conflict { .. })
    ));
    assert!(matches!(
        s.update(promote.clone(), 0).await,
        Err(PortStorageError::Duplicate { .. })
    ));
    s.soft_delete("org_1", "ws_default")
        .await
        .expect("delete prior default");
    promote.version = 1;
    s.update(promote, 0)
        .await
        .expect("promote after prior default deletion");

    s.soft_delete("org_1", "ws_1").await.expect("soft_delete");
    assert!(s.get("org_1", "ws_1").await.unwrap().is_none());
    assert!(s.get_by_slug("org_1", "main").await.unwrap().is_none());
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
        .upsert_org_member_guarded(org_member("org_a", "user", OrgMembershipRole::Owner))
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

async fn assert_workspace_member_listing_and_org_removal_cleanup(b: &dyn IdentityBackend) {
    let orgs = b.org_store().await;
    let workspaces = b.workspace_store().await;
    let store = b.membership_store().await;
    orgs.create(org_row("org", "org")).await.unwrap();
    workspaces
        .create(workspace_row("active", "org", "active"))
        .await
        .unwrap();
    workspaces
        .create(workspace_row("deleted", "org", "deleted"))
        .await
        .unwrap();
    store
        .upsert_org_member_guarded(org_member("org", "owner", OrgMembershipRole::Owner))
        .await
        .unwrap();
    store
        .upsert_org_member_guarded(org_member("org", "admin", OrgMembershipRole::Admin))
        .await
        .unwrap();

    let mut service_org_member = org_member("org", "service", OrgMembershipRole::Member);
    service_org_member.principal_kind = PrincipalKind::ServiceAccount;
    store
        .upsert_org_member_guarded(service_org_member)
        .await
        .unwrap();

    let mut service = workspace_member("org", "active", "service");
    service.principal_kind = PrincipalKind::ServiceAccount;
    service.role = WorkspaceMembershipRole::Viewer;
    store.upsert_workspace_member(service).await.unwrap();
    let mut owner_active = workspace_member("org", "active", "owner");
    owner_active.role = WorkspaceMembershipRole::Admin;
    store.upsert_workspace_member(owner_active).await.unwrap();
    store
        .upsert_workspace_member(workspace_member("org", "deleted", "owner"))
        .await
        .unwrap();

    let listed = store.list_workspace_members("org", "active").await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].principal_kind, PrincipalKind::ServiceAccount);
    assert_eq!(listed[0].principal_id, "service");
    assert_eq!(listed[0].role, WorkspaceMembershipRole::Viewer);
    assert_eq!(listed[1].principal_kind, PrincipalKind::User);
    assert_eq!(listed[1].principal_id, "owner");
    assert_eq!(listed[1].role, WorkspaceMembershipRole::Admin);

    workspaces.soft_delete("org", "deleted").await.unwrap();
    assert!(matches!(
        store.list_workspace_members("org", "deleted").await,
        Err(PortStorageError::NotFound { .. })
    ));
    assert_eq!(
        store
            .remove_org_member_guarded("org", PrincipalKind::User, "owner")
            .await
            .unwrap(),
        OrgMemberRemoveOutcome::Removed
    );
    for workspace_id in ["active", "deleted"] {
        assert!(
            store
                .get(
                    ScopeKind::Workspace,
                    workspace_id,
                    PrincipalKind::User,
                    "owner"
                )
                .await
                .unwrap()
                .is_none(),
            "org removal must clear grants for {workspace_id}"
        );
    }
    assert_eq!(
        store
            .list_workspace_members("org", "active")
            .await
            .unwrap()
            .len(),
        1
    );
    store
        .upsert_org_member_guarded(org_member("org", "owner", OrgMembershipRole::Member))
        .await
        .unwrap();
    assert_eq!(
        store
            .list_workspace_members("org", "active")
            .await
            .unwrap()
            .len(),
        1
    );
    orgs.soft_delete("org").await.unwrap();
    assert!(matches!(
        store.list_workspace_members("org", "active").await,
        Err(PortStorageError::NotFound { .. })
    ));
}

async fn assert_workspace_upsert_requires_org_membership_and_serializes_removal(
    b: &dyn IdentityBackend,
) {
    let orgs = b.org_store().await;
    let workspaces = b.workspace_store().await;
    let store = b.membership_store().await;
    orgs.create(org_row("org", "org")).await.unwrap();
    workspaces
        .create(workspace_row("ws", "org", "ws"))
        .await
        .unwrap();
    store
        .upsert_org_member_guarded(org_member("org", "admin", OrgMembershipRole::Admin))
        .await
        .unwrap();

    assert!(matches!(
        store
            .upsert_workspace_member(workspace_member("org", "ws", "absent"))
            .await,
        Err(PortStorageError::NotFound { .. })
    ));
    assert!(
        store
            .get(ScopeKind::Workspace, "ws", PrincipalKind::User, "absent")
            .await
            .unwrap()
            .is_none()
    );

    store
        .upsert_org_member_guarded(org_member("org", "target", OrgMembershipRole::Member))
        .await
        .unwrap();
    let removal_store = Arc::clone(&store);
    let upsert_store = Arc::clone(&store);
    let (removal, upsert) = tokio::join!(
        removal_store.remove_org_member_guarded("org", PrincipalKind::User, "target"),
        upsert_store.upsert_workspace_member(workspace_member("org", "ws", "target"))
    );
    assert_eq!(removal.unwrap(), OrgMemberRemoveOutcome::Removed);
    assert!(upsert.is_ok() || matches!(upsert, Err(PortStorageError::NotFound { .. })));
    assert!(
        store
            .get(ScopeKind::Org, "org", PrincipalKind::User, "target")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get(ScopeKind::Workspace, "ws", PrincipalKind::User, "target")
            .await
            .unwrap()
            .is_none(),
        "removal must reject or cascade a concurrent workspace grant"
    );
}

async fn assert_ambiguous_workspace_blocks_org_removal(b: &dyn IdentityBackend) {
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
        .upsert_org_member_guarded(org_member("org_a", "owner", OrgMembershipRole::Owner))
        .await
        .unwrap();
    store
        .upsert_org_member_guarded(org_member("org_a", "admin", OrgMembershipRole::Admin))
        .await
        .unwrap();
    store
        .upsert_workspace_member(workspace_member("org_a", "shared", "owner"))
        .await
        .unwrap();
    workspaces
        .create(workspace_row("shared", "org_b", "b"))
        .await
        .unwrap();
    workspaces.soft_delete("org_b", "shared").await.unwrap();

    assert!(matches!(
        store.list_workspace_members("org_a", "shared").await,
        Err(PortStorageError::NotFound { .. })
    ));
    assert!(matches!(
        store
            .remove_org_member_guarded("org_a", PrincipalKind::User, "owner")
            .await,
        Err(PortStorageError::Serialization(_))
    ));
    assert!(
        store
            .get(ScopeKind::Org, "org_a", PrincipalKind::User, "owner")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .get(ScopeKind::Workspace, "shared", PrincipalKind::User, "owner")
            .await
            .unwrap()
            .is_some()
    );
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

async fn assert_tenant_provisioning(b: &dyn IdentityBackend) {
    let store = b.tenant_provisioning_store().await;
    let orgs = b.org_store().await;
    let workspaces = b.workspace_store().await;
    let memberships = b.membership_store().await;

    let request = tenant_request("org_bootstrap", "bootstrap", "ws_bootstrap");
    let (left, right) = tokio::join!(
        store.provision_tenant(request.clone()),
        store.provision_tenant(request.clone())
    );
    let mut outcomes = [left.unwrap(), right.unwrap()];
    outcomes.sort_by_key(|outcome| match outcome {
        TenantProvisioningOutcome::Created => 0,
        TenantProvisioningOutcome::Replayed => 1,
        TenantProvisioningOutcome::Conflict(_) => 2,
    });
    let observed_org = orgs.get("org_bootstrap").await.unwrap();
    let observed_workspace = workspaces
        .get("org_bootstrap", "ws_bootstrap")
        .await
        .unwrap();
    let observed_owner = memberships
        .get(
            ScopeKind::Org,
            "org_bootstrap",
            PrincipalKind::User,
            "owner",
        )
        .await
        .unwrap();
    assert_eq!(
        outcomes,
        [
            TenantProvisioningOutcome::Created,
            TenantProvisioningOutcome::Replayed
        ],
        "persisted org={observed_org:?}, workspace={observed_workspace:?}, owner={observed_owner:?}"
    );
    let persisted_org = orgs.get("org_bootstrap").await.unwrap().unwrap();
    assert!(request.org().matches_persisted(&persisted_org));
    let persisted_workspace = workspaces
        .get("org_bootstrap", "ws_bootstrap")
        .await
        .unwrap()
        .unwrap();
    assert!(
        request
            .default_workspace()
            .matches_persisted("org_bootstrap", &persisted_workspace)
    );
    let owner_before = memberships
        .get(
            ScopeKind::Org,
            "org_bootstrap",
            PrincipalKind::User,
            "owner",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owner_before.role, OrgMembershipRole::Owner.as_str());
    assert_eq!(owner_before.added_by.as_deref(), Some("bootstrap"));
    assert_eq!(
        store.provision_tenant(request.clone()).await.unwrap(),
        TenantProvisioningOutcome::Replayed
    );
    let owner_after = memberships
        .get(
            ScopeKind::Org,
            "org_bootstrap",
            PrincipalKind::User,
            "owner",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owner_after.added_at, owner_before.added_at);

    memberships
        .upsert_org_member_guarded(org_member(
            "org_bootstrap",
            "second-admin",
            OrgMembershipRole::Admin,
        ))
        .await
        .unwrap();
    memberships
        .upsert_org_member_guarded(org_member(
            "org_bootstrap",
            "owner",
            OrgMembershipRole::Member,
        ))
        .await
        .unwrap();
    assert_eq!(
        store.provision_tenant(request.clone()).await.unwrap(),
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState)
    );
    assert_eq!(
        memberships
            .get(
                ScopeKind::Org,
                "org_bootstrap",
                PrincipalKind::User,
                "owner"
            )
            .await
            .unwrap()
            .unwrap()
            .role,
        OrgMembershipRole::Member.as_str()
    );

    let same_slug_org = TenantOrgCreate::new(
        "org_other".into(),
        "bootstrap".into(),
        "Test Org".into(),
        "usr_1".into(),
        "free".into(),
        None,
        serde_json::json!({}),
    )
    .unwrap();
    let same_slug_workspace = TenantDefaultWorkspaceCreate::new(
        "ws_other".into(),
        "default".into(),
        "Test Workspace".into(),
        None,
        "usr_1".into(),
        serde_json::json!({}),
    )
    .unwrap();
    let same_slug = TenantProvisioningRequest::new(
        same_slug_org,
        same_slug_workspace,
        PrincipalKind::User,
        "other-owner".into(),
        Some("bootstrap".into()),
    )
    .unwrap();
    assert_eq!(
        store.provision_tenant(same_slug).await.unwrap(),
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState)
    );
    assert!(orgs.get("org_other").await.unwrap().is_none());

    let collision_request = tenant_request("org_extra_default", "extra-default", "ws_primary");
    assert_eq!(
        store
            .provision_tenant(collision_request.clone())
            .await
            .unwrap(),
        TenantProvisioningOutcome::Created
    );
    let mut extra_default = workspace_row("ws_extra", "org_extra_default", "extra");
    extra_default.is_default = true;
    assert!(matches!(
        workspaces.create(extra_default).await,
        Err(PortStorageError::Duplicate { .. })
    ));
    assert_eq!(
        store.provision_tenant(collision_request).await.unwrap(),
        TenantProvisioningOutcome::Replayed
    );

    let id_collision_request =
        tenant_request("org_workspace_id", "workspace-id", "ws_global_collision");
    assert_eq!(
        store
            .provision_tenant(id_collision_request.clone())
            .await
            .unwrap(),
        TenantProvisioningOutcome::Created
    );
    orgs.create(org_row("org_workspace_alias", "workspace-alias"))
        .await
        .unwrap();
    workspaces
        .create(workspace_row(
            "ws_global_collision",
            "org_workspace_alias",
            "alias",
        ))
        .await
        .unwrap();
    assert_eq!(
        store.provision_tenant(id_collision_request).await.unwrap(),
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState)
    );

    let race_a = tenant_request("org_race_a", "race-a", "ws_race_shared");
    let race_b = tenant_request("org_race_b", "race-b", "ws_race_shared");
    let (race_a_outcome, race_b_outcome) = tokio::join!(
        store.provision_tenant(race_a),
        store.provision_tenant(race_b)
    );
    let race_outcomes = [race_a_outcome.unwrap(), race_b_outcome.unwrap()];
    assert_eq!(
        race_outcomes
            .iter()
            .filter(|outcome| **outcome == TenantProvisioningOutcome::Created)
            .count(),
        1
    );
    assert_eq!(
        race_outcomes
            .iter()
            .filter(|outcome| {
                **outcome
                    == TenantProvisioningOutcome::Conflict(
                        TenantProvisioningConflict::ExistingState,
                    )
            })
            .count(),
        1
    );

    orgs.create(org_row("org_partial", "partial"))
        .await
        .unwrap();
    let partial = tenant_request("org_partial", "partial", "ws_partial");
    assert_eq!(
        store.provision_tenant(partial).await.unwrap(),
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState)
    );
    assert!(
        workspaces
            .get("org_partial", "ws_partial")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        memberships
            .list_for_scope(ScopeKind::Org, "org_partial")
            .await
            .unwrap()
            .is_empty()
    );

    assert!(
        TenantDefaultWorkspaceCreate::new(
            String::new(),
            "default".into(),
            "Test Workspace".into(),
            None,
            "usr_1".into(),
            serde_json::json!({}),
        )
        .is_err()
    );
    assert!(orgs.get("org_invalid").await.unwrap().is_none());
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
identity_matrix!(
    workspace_member_listing_and_org_removal_cleanup,
    assert_workspace_member_listing_and_org_removal_cleanup
);
identity_matrix!(
    ambiguous_workspace_blocks_org_removal,
    assert_ambiguous_workspace_blocks_org_removal
);
identity_matrix!(
    workspace_upsert_requires_org_membership_and_serializes_removal,
    assert_workspace_upsert_requires_org_membership_and_serializes_removal
);
identity_matrix!(membership_lockout, assert_membership_lockout);
identity_matrix!(tenant_provisioning, assert_tenant_provisioning);
identity_matrix!(resource_store_contract, assert_resource_contract);
identity_matrix!(trigger_store_contract, assert_trigger_contract);
identity_matrix!(quota_store_contract, assert_quota_contract);
identity_matrix!(audit_store_contract, assert_audit_contract);
identity_matrix!(blob_store_contract, assert_blob_contract);

/// Provisioning and ordinary workspace writes use the same per-org lock.
/// Whichever transaction wins, the organization can retain only one live
/// default workspace.
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_provisioning_serializes_with_workspace_create() {
    if postgres_skip().is_some() {
        return;
    }
    let backend = PostgresBackend::default();
    let pool = backend.pool().await;
    let provisioning = backend.tenant_provisioning_store().await;
    let workspaces = backend.workspace_store().await;
    let request = tenant_request("org_default_race", "default-race", "ws_provisioned");
    let mut competing = workspace_row("ws_competing", "org_default_race", "competing");
    competing.is_default = true;

    let (provisioning_result, workspace_result) = tokio::join!(
        provisioning.provision_tenant(request),
        workspaces.create(competing)
    );
    match (provisioning_result, workspace_result) {
        (Ok(TenantProvisioningOutcome::Created), Err(PortStorageError::Duplicate { .. }))
        | (
            Ok(TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState)),
            Ok(()),
        ) => {},
        outcomes => panic!("unexpected provisioning/workspace race outcomes: {outcomes:?}"),
    }

    let active_defaults: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM port_workspaces \
         WHERE org_id = $1 AND is_default = TRUE AND deleted_at IS NULL",
    )
    .bind("org_default_race")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active_defaults, 1);
}

/// Alias creation and membership cleanup share the workspace-id lock. This
/// prevents a new cross-org alias from appearing between the cascade's
/// ambiguity check and its deletion of grants keyed only by workspace id.
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_workspace_alias_create_serializes_with_membership_cascade() {
    if postgres_skip().is_some() {
        return;
    }
    let backend = PostgresBackend::default();
    let pool = backend.pool().await;
    let orgs = backend.org_store().await;
    let workspaces = backend.workspace_store().await;
    let memberships = backend.membership_store().await;

    orgs.create(org_row("org_alias_source", "alias-source"))
        .await
        .unwrap();
    orgs.create(org_row("org_alias_target", "alias-target"))
        .await
        .unwrap();
    workspaces
        .create(workspace_row("ws_alias_race", "org_alias_source", "source"))
        .await
        .unwrap();
    memberships
        .upsert_org_member_guarded(org_member(
            "org_alias_source",
            "target",
            OrgMembershipRole::Owner,
        ))
        .await
        .unwrap();
    memberships
        .upsert_org_member_guarded(org_member(
            "org_alias_source",
            "admin",
            OrgMembershipRole::Admin,
        ))
        .await
        .unwrap();
    memberships
        .upsert_workspace_member(workspace_member(
            "org_alias_source",
            "ws_alias_race",
            "target",
        ))
        .await
        .unwrap();

    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind("tenant-workspace-id:ws_alias_race")
        .execute(&mut *blocker)
        .await
        .unwrap();

    let alias_create =
        workspaces.create(workspace_row("ws_alias_race", "org_alias_target", "target"));
    let cascade =
        memberships.remove_org_member_guarded("org_alias_source", PrincipalKind::User, "target");
    tokio::pin!(alias_create, cascade);

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), &mut alias_create)
            .await
            .is_err(),
        "alias creation must wait for the workspace-id lock"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), &mut cascade)
            .await
            .is_err(),
        "membership cascade must wait for the workspace-id lock"
    );
    blocker.commit().await.unwrap();

    let (alias_result, cascade_result) = tokio::join!(alias_create, cascade);
    alias_result.unwrap();
    match cascade_result {
        Ok(OrgMemberRemoveOutcome::Removed) => {
            assert!(
                memberships
                    .get(
                        ScopeKind::Org,
                        "org_alias_source",
                        PrincipalKind::User,
                        "target"
                    )
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                memberships
                    .get(
                        ScopeKind::Workspace,
                        "ws_alias_race",
                        PrincipalKind::User,
                        "target"
                    )
                    .await
                    .unwrap()
                    .is_none()
            );
        },
        Err(PortStorageError::Serialization(_)) => {
            assert!(
                memberships
                    .get(
                        ScopeKind::Org,
                        "org_alias_source",
                        PrincipalKind::User,
                        "target"
                    )
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                memberships
                    .get(
                        ScopeKind::Workspace,
                        "ws_alias_race",
                        PrincipalKind::User,
                        "target"
                    )
                    .await
                    .unwrap()
                    .is_some()
            );
        },
        outcome => panic!("unexpected membership cascade outcome: {outcome:?}"),
    }
}

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
    assert!(matches!(
        store.list_workspace_members("org", "ws").await,
        Err(nebula_storage_port::StorageError::Serialization(_))
    ));
}
