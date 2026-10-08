//! Identity-zoo behavioral conformance matrix (spec-16 §5 / §9, §6.1) on the
//! in-memory and SQLite adapters.
//!
//! One backend-agnostic contract suite for the tenant directory and workspace
//! objects (`Org`, `Workspace`, `Membership`, tenant provisioning, `Resource`,
//! `Trigger`); the shared assertions encode the abstract contract
//! every adapter must satisfy. The PostgreSQL arm lives in
//! `identity_conformance_postgres` (an evidence binary that needs a live
//! database).
//!
//! Skip-clean policy: the SQLite case skips when built without
//! `--features sqlite`. A skipped backend prints a WARN and passes.

#![expect(
    clippy::print_stderr,
    reason = "conformance harness reports skip/diagnostic lines to stderr"
)]

use std::future::Future;

use rstest::rstest;

include!("support/identity_conformance_common.rs");

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
    #[cfg(feature = "sqlite")]
    async fn seed_trigger_parents(&self, scope: &Scope, workflow_id: &str) {
        use execution_parents::SeedExecutionParents as _;
        self.pool()
            .await
            .seed_execution_parents(scope, workflow_id)
            .await;
    }
    #[cfg(feature = "sqlite")]
    async fn seed_resource_parents(&self, scope: &Scope) {
        execution_parents::provision_scope(self.tenant_provisioning_store().await.as_ref(), scope)
            .await;
    }
}

#[cfg(feature = "sqlite")]
#[path = "support/execution_parents.rs"]
mod execution_parents;

fn sqlite_skip() -> Option<&'static str> {
    if cfg!(feature = "sqlite") {
        None
    } else {
        Some("SQLite identity case skipped — built without `--features sqlite`")
    }
}

fn skip_reason(backend: &dyn IdentityBackend) -> Option<&'static str> {
    match backend.name() {
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

// ── matrix ────────────────────────────────────────────────────────────────

macro_rules! identity_matrix {
    ($name:ident, $assertion:path) => {
        #[rstest]
        #[case::in_memory(in_memory())]
        #[case::sqlite(sqlite())]
        #[tokio::test]
        async fn $name(#[case] backend: Box<dyn IdentityBackend>) {
            run(backend, |b| async move { $assertion(b.as_ref()).await }).await;
        }
    };
}

identity_matrix!(org_store_contract, assert_org_contract);
identity_matrix!(workspace_store_contract, assert_workspace_contract);
identity_matrix!(membership_store_contract, assert_membership_contract);
identity_matrix!(membership_snapshot, assert_membership_snapshot);
identity_matrix!(
    deleted_org_hides_its_memberships,
    assert_deleted_org_hides_its_memberships
);
identity_matrix!(
    workspace_member_listing_and_org_removal_cleanup,
    assert_workspace_member_listing_and_org_removal_cleanup
);
identity_matrix!(
    workspace_upsert_requires_org_membership_and_serializes_removal,
    assert_workspace_upsert_requires_org_membership_and_serializes_removal
);
identity_matrix!(membership_lockout, assert_membership_lockout);
identity_matrix!(tenant_provisioning, assert_tenant_provisioning);
identity_matrix!(
    tenant_provisioning_competing_commands,
    assert_tenant_provisioning_competing_commands
);
identity_matrix!(
    tenant_provisioning_history,
    assert_tenant_provisioning_history
);
identity_matrix!(
    tenant_provisioning_json_identity,
    assert_tenant_provisioning_json_identity
);
identity_matrix!(resource_store_contract, assert_resource_contract);
identity_matrix!(trigger_store_contract, assert_trigger_contract);

/// A completed provisioning command must never recreate a purged tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn tenant_provisioning_replay_after_purge_sqlite() {
    let backend = SqliteBackend::default();
    let store = backend.tenant_provisioning_store().await;
    let request = tenant_request("purged_org", "purged", "purged_workspace");
    assert_eq!(
        store.provision_tenant(request.clone()).await.unwrap(),
        TenantProvisioningOutcome::Created
    );
    // Arrange the result of tenant purge; the assertion uses public ports.
    sqlx::query("DELETE FROM orgs WHERE id = 'purged_org'")
        .execute(&backend.pool().await)
        .await
        .unwrap();
    assert_eq!(
        store.provision_tenant(request).await.unwrap(),
        TenantProvisioningOutcome::Replayed,
        "replay acknowledges the original acceptance, not a second creation"
    );
    assert!(
        backend
            .org_store()
            .await
            .get("purged_org")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .workspace_store()
            .await
            .get("purged_org", "purged_workspace")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        backend
            .membership_store()
            .await
            .get_tenant_membership(
                "purged_org",
                Some("purged_workspace"),
                PrincipalKind::User,
                "owner"
            )
            .await
            .unwrap(),
        TenantMembershipSnapshot {
            org_role: None,
            workspace_role: None
        }
    );
    assert_eq!(
        store
            .provision_tenant(tenant_request("another_org", "another", "purged_workspace"))
            .await
            .unwrap(),
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState),
        "the initial workspace identity also stays reserved after purge"
    );
    assert!(
        backend
            .org_store()
            .await
            .get("another_org")
            .await
            .unwrap()
            .is_none()
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn tenant_provisioning_receipt_failure_rolls_back_sqlite() {
    let backend = SqliteBackend::default();
    let pool = backend.pool().await;
    let store = backend.tenant_provisioning_store().await;
    let request = tenant_request("rollback_org", "rollback", "rollback_ws");
    sqlx::raw_sql(
        "CREATE TRIGGER reject_receipt BEFORE INSERT ON tenant_provisioning_receipts
        BEGIN SELECT RAISE(ABORT, 'receipt unavailable'); END;",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(store.provision_tenant(request.clone()).await.is_err());
    assert!(
        backend
            .org_store()
            .await
            .get("rollback_org")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .workspace_store()
            .await
            .get("rollback_org", "rollback_ws")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        backend
            .membership_store()
            .await
            .get_tenant_membership("rollback_org", None, PrincipalKind::User, "owner")
            .await
            .unwrap()
            .org_role,
        None
    );
    sqlx::query("DROP TRIGGER reject_receipt")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        store.provision_tenant(request).await.unwrap(),
        TenantProvisioningOutcome::Created
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn tenant_provisioning_receipt_survives_file_reopen() {
    use nebula_storage::sqlite::{SqliteOrgStore, SqliteTenantProvisioningStore};

    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("tenant.db"))
        .create_if_missing(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let request = tenant_request("persistent_org", "persistent", "persistent_ws");
    assert_eq!(
        SqliteTenantProvisioningStore::new(pool.clone())
            .provision_tenant(request.clone())
            .await
            .unwrap(),
        TenantProvisioningOutcome::Created
    );
    pool.close().await;

    let reopened = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&reopened)
        .await
        .unwrap();
    let store = SqliteTenantProvisioningStore::new(reopened.clone());
    assert_eq!(
        store.provision_tenant(request.clone()).await.unwrap(),
        TenantProvisioningOutcome::Replayed
    );
    sqlx::query("DELETE FROM orgs WHERE id = 'persistent_org'")
        .execute(&reopened)
        .await
        .unwrap();
    reopened.close().await;

    let reopened = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&reopened)
        .await
        .unwrap();
    assert_eq!(
        SqliteTenantProvisioningStore::new(reopened.clone())
            .provision_tenant(request)
            .await
            .unwrap(),
        TenantProvisioningOutcome::Replayed
    );
    assert!(
        SqliteOrgStore::new(reopened.clone())
            .get("persistent_org")
            .await
            .unwrap()
            .is_none()
    );
    reopened.close().await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn tenant_provisioning_migration_seals_preexisting_sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations/sqlite")
        .run_to(8, &pool)
        .await
        .unwrap();
    let orgs = nebula_storage::sqlite::SqliteOrgStore::new(pool.clone());
    orgs.create(org_row("preexisting", "preexisting"))
        .await
        .unwrap();
    let before = orgs.get("preexisting").await.unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    assert_eq!(orgs.get("preexisting").await.unwrap(), before);
    let store = nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone());
    let request = tenant_request("preexisting", "preexisting", "preexisting_ws");
    assert_eq!(
        store.provision_tenant(request.clone()).await.unwrap(),
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::PreexistingTenant)
    );
    sqlx::query("DELETE FROM orgs WHERE id = 'preexisting'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        store.provision_tenant(request).await.unwrap(),
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::PreexistingTenant)
    );
    assert!(orgs.get("preexisting").await.unwrap().is_none());
    pool.close().await;
}

/// A frozen v1 receipt must remain readable even if the current encoder changes.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn tenant_provisioning_reads_historical_v1_sqlite() {
    let backend = SqliteBackend::default();
    let pool = backend.pool().await;
    let digest =
        hex::decode("4d693d7b7db0e8411c230bc5627d512e133445adc3ba96480274df8c4a916b69").unwrap();
    sqlx::query(
        "INSERT INTO tenant_provisioning_receipts
        (org_id, initial_workspace_id, request_version, request_digest, recorded_at)
        VALUES ('json_org', 'json_ws', 1, ?1, 0)",
    )
    .bind(digest)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        backend
            .tenant_provisioning_store()
            .await
            .provision_tenant(tenant_json_request(
                serde_json::json!({"a": true, "b": [{"x": 1, "y": 2}]})
            ))
            .await
            .unwrap(),
        TenantProvisioningOutcome::Replayed
    );
    assert!(
        backend
            .org_store()
            .await
            .get("json_org")
            .await
            .unwrap()
            .is_none()
    );
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

/// Roles and principal kinds are closed vocabularies in the schema: a row
/// outside them cannot be written, whatever the writer.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn membership_vocabularies_are_closed_by_the_schema_sqlite() {
    let backend = SqliteBackend::default();
    let pool = backend.pool().await;
    let orgs = backend.org_store().await;
    let workspaces = backend.workspace_store().await;
    let store = backend.membership_store().await;
    orgs.create(org_row("org", "org")).await.unwrap();
    workspaces
        .create(workspace_row("ws", "org", "ws"))
        .await
        .unwrap();
    store
        .upsert_org_member_guarded(org_member("org", "owner", OrgMembershipRole::Owner))
        .await
        .unwrap();
    store
        .upsert_workspace_member(workspace_member("org", "ws", "owner"))
        .await
        .unwrap();
    for statement in [
        "UPDATE org_memberships SET role = 'unknown'",
        "UPDATE org_memberships SET role = 'WorkspaceAdmin'",
        "UPDATE org_memberships SET principal_kind = 'robot'",
        "UPDATE workspace_memberships SET role = 'OrgOwner'",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "the schema must reject `{statement}`"
        );
    }
    assert_eq!(
        store
            .get_tenant_membership("org", Some("ws"), PrincipalKind::User, "owner")
            .await
            .unwrap(),
        TenantMembershipSnapshot {
            org_role: Some(OrgMembershipRole::Owner),
            workspace_role: Some(WorkspaceMembershipRole::Editor),
        }
    );
}
