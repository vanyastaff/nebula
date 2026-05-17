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

use std::future::Future;
use std::sync::Arc;

use nebula_storage_port::Scope;
use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, QuotaRow, ResourceRow, TriggerRow, UserRow,
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
struct InMemoryBackend;

#[async_trait::async_trait]
impl IdentityBackend for InMemoryBackend {
    fn name(&self) -> &'static str {
        "InMemory"
    }
    async fn user_store(&self) -> Arc<dyn UserStore> {
        Arc::new(nebula_storage::inmem::InMemoryUserStore::new())
    }
    async fn org_store(&self) -> Arc<dyn OrgStore> {
        Arc::new(nebula_storage::inmem::InMemoryOrgStore::new())
    }
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore> {
        Arc::new(nebula_storage::inmem::InMemoryWorkspaceStore::new())
    }
    async fn membership_store(&self) -> Arc<dyn MembershipStore> {
        Arc::new(nebula_storage::inmem::InMemoryMembershipStore::new())
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

#[derive(Default)]
struct SqliteBackend;

#[async_trait::async_trait]
impl IdentityBackend for SqliteBackend {
    fn name(&self) -> &'static str {
        "Sqlite(:memory:)"
    }
    async fn user_store(&self) -> Arc<dyn UserStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn org_store(&self) -> Arc<dyn OrgStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn membership_store(&self) -> Arc<dyn MembershipStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn resource_store(&self) -> Arc<dyn ResourceStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn trigger_store(&self) -> Arc<dyn TriggerStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn quota_store(&self) -> Arc<dyn QuotaStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn audit_store(&self) -> Arc<dyn AuditStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
    async fn blob_store(&self) -> Arc<dyn BlobStore> {
        unimplemented!("SQLite identity adapter lands in a follow-up commit")
    }
}

// ── Postgres backend (DATABASE_URL-gated) ─────────────────────────────────

#[derive(Default)]
struct PostgresBackend;

#[async_trait::async_trait]
impl IdentityBackend for PostgresBackend {
    fn name(&self) -> &'static str {
        "Postgres"
    }
    async fn user_store(&self) -> Arc<dyn UserStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn org_store(&self) -> Arc<dyn OrgStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn membership_store(&self) -> Arc<dyn MembershipStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn resource_store(&self) -> Arc<dyn ResourceStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn trigger_store(&self) -> Arc<dyn TriggerStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn quota_store(&self) -> Arc<dyn QuotaStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn audit_store(&self) -> Arc<dyn AuditStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
    }
    async fn blob_store(&self) -> Arc<dyn BlobStore> {
        unimplemented!("Postgres identity adapter lands in a follow-up commit")
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
    Box::new(InMemoryBackend)
}

fn sqlite() -> Box<dyn IdentityBackend> {
    Box::new(SqliteBackend)
}

fn postgres() -> Box<dyn IdentityBackend> {
    Box::new(PostgresBackend)
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
        mfa_secret: None,
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

fn membership_row(scope_id: &str, principal_id: &str) -> MembershipRow {
    MembershipRow {
        scope_kind: "org".into(),
        scope_id: scope_id.into(),
        principal_kind: "user".into(),
        principal_id: principal_id.into(),
        role: "admin".into(),
        added_at: "2026-01-01T00:00:00Z".into(),
        added_by: None,
    }
}

fn resource_row(id: &str, workspace_id: &str, slug: &str) -> ResourceRow {
    ResourceRow {
        id: id.into(),
        workspace_id: workspace_id.into(),
        slug: slug.into(),
        display_name: "Test Resource".into(),
        kind: "http".into(),
        config: serde_json::json!({}),
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
    let s = b.membership_store().await;
    s.upsert(membership_row("org_1", "usr_1"))
        .await
        .expect("upsert");
    let mut promoted = membership_row("org_1", "usr_1");
    promoted.role = "owner".into();
    s.upsert(promoted).await.expect("upsert replaces");
    assert_eq!(
        s.get("org", "org_1", "user", "usr_1")
            .await
            .unwrap()
            .unwrap()
            .role,
        "owner"
    );
    assert_eq!(s.list_for_scope("org", "org_1").await.unwrap().len(), 1);
    // cross-scope read is a miss
    assert!(
        s.get("org", "org_2", "user", "usr_1")
            .await
            .unwrap()
            .is_none()
    );
    s.remove("org", "org_1", "user", "usr_1")
        .await
        .expect("remove");
    assert!(
        s.get("org", "org_1", "user", "usr_1")
            .await
            .unwrap()
            .is_none()
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
    assert_eq!(s.list(&a).await.unwrap().len(), 1);
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
identity_matrix!(resource_store_contract, assert_resource_contract);
identity_matrix!(trigger_store_contract, assert_trigger_contract);
identity_matrix!(quota_store_contract, assert_quota_contract);
identity_matrix!(audit_store_contract, assert_audit_contract);
identity_matrix!(blob_store_contract, assert_blob_contract);
