//! Identity-zoo behavioral conformance matrix on the PostgreSQL adapter.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one (`PostgresBackend::pool` panics naming `DATABASE_URL`), so a green
//! run always means the cases ran. Excluded from the default nextest profile and
//! collected by the CI `postgres-conformance` job. The in-memory and SQLite arms
//! live in `identity_conformance`.

#![cfg(feature = "postgres")]

use rstest::rstest;

include!("support/identity_conformance_common.rs");

fn postgres() -> Box<dyn IdentityBackend> {
    Box::new(PostgresBackend::default())
}

// ── Postgres backend (DATABASE_URL-gated) ─────────────────────────────────

/// Each `PostgresBackend` instance owns one pool created lazily on first
/// store request (port schema installed once). Requires `DATABASE_URL`; the
/// first store request panics naming it when it is absent.
///
/// The pool is pinned to a private schema, matching the fresh-store contract
/// the InMemory and SQLite backends give every case — see
/// `tests/support/postgres_schema.rs` for why a shared `public` schema
/// silently invalidated the Postgres arm.
#[derive(Default)]
struct PostgresBackend {
    pool: tokio::sync::OnceCell<sqlx::PgPool>,
}

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

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
    async fn org_store(&self) -> Arc<dyn OrgStore> {
        Arc::new(nebula_storage::postgres::PgOrgStore::new(self.pool().await))
    }
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore> {
        Arc::new(nebula_storage::postgres::PgWorkspaceStore::new(
            self.pool().await,
        ))
    }
    async fn membership_store(&self) -> Arc<dyn MembershipStore> {
        Arc::new(nebula_storage::postgres::PgMembershipStore::new(
            self.pool().await,
        ))
    }
    async fn tenant_provisioning_store(&self) -> Arc<dyn TenantProvisioningStore> {
        Arc::new(nebula_storage::postgres::PgTenantProvisioningStore::new(
            self.pool().await,
        ))
    }
    async fn resource_store(&self) -> Arc<dyn ResourceStore> {
        Arc::new(nebula_storage::postgres::PgResourceStore::new(
            self.pool().await,
        ))
    }
    async fn trigger_store(&self) -> Arc<dyn TriggerStore> {
        Arc::new(nebula_storage::postgres::PgTriggerStore::new(
            self.pool().await,
        ))
    }
    async fn seed_trigger_parents(&self, scope: &Scope, workflow_id: &str) {
        use execution_parents::SeedExecutionParents as _;
        self.pool()
            .await
            .seed_execution_parents(scope, workflow_id)
            .await;
    }
    async fn seed_resource_parents(&self, scope: &Scope) {
        execution_parents::provision_scope(self.tenant_provisioning_store().await.as_ref(), scope)
            .await;
    }
}

#[path = "support/execution_parents.rs"]
mod execution_parents;

// ── matrix ────────────────────────────────────────────────────────────────

macro_rules! identity_matrix {
    ($name:ident, $assertion:path) => {
        #[rstest]
        #[case::postgres(postgres())]
        #[tokio::test]
        async fn $name(#[case] backend: Box<dyn IdentityBackend>) {
            assert_eq!(backend.name(), "Postgres");
            $assertion(backend.as_ref()).await;
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
identity_matrix!(resource_store_contract, assert_resource_contract);
identity_matrix!(trigger_store_contract, assert_trigger_contract);

/// Provisioning races an ordinary default-workspace create. Whichever
/// transaction wins, the organization retains only one live default
/// workspace (`uq_workspaces__org_id__live_default`).
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_provisioning_serializes_with_workspace_create() {
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
    // The competing create either precedes the org (no parent) or follows
    // the provisioned default.
    match (provisioning_result, workspace_result) {
        (
            Ok(TenantProvisioningOutcome::Created),
            Err(PortStorageError::Duplicate { .. } | PortStorageError::NotFound { .. }),
        ) => {},
        outcomes => panic!("unexpected provisioning/workspace race outcomes: {outcomes:?}"),
    }

    let active_defaults: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workspaces \
         WHERE org_id = $1 AND is_default = TRUE AND deleted_at IS NULL",
    )
    .bind("org_default_race")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active_defaults, 1);
}

/// Seed one org with a live workspace and an owner grant for `principal`.
#[cfg(feature = "postgres")]
async fn seed_workspace(backend: &PostgresBackend, org: &str, workspace: &str, principal: &str) {
    backend
        .org_store()
        .await
        .create(org_row(org, org))
        .await
        .unwrap();
    backend
        .workspace_store()
        .await
        .create(workspace_row(workspace, org, workspace))
        .await
        .unwrap();
    backend
        .membership_store()
        .await
        .upsert_org_member_guarded(org_member(org, principal, OrgMembershipRole::Owner))
        .await
        .unwrap();
}

async fn workspace_grant_count(pool: &sqlx::PgPool, workspace: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM workspace_memberships WHERE workspace_id = $1")
        .bind(workspace)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A workspace grant serializes with a concurrent soft delete of its
/// workspace: while the delete is uncommitted the grant waits on the
/// workspace row, and once it commits the grant is refused — never written
/// beneath a deleted workspace.
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_workspace_grant_waits_for_a_concurrent_workspace_delete() {
    let backend = PostgresBackend::default();
    let pool = backend.pool().await;
    seed_workspace(&backend, "org_ws_race", "ws_race", "owner").await;
    let memberships = backend.membership_store().await;

    let mut deleter = pool.begin().await.unwrap();
    sqlx::query("UPDATE workspaces SET deleted_at = now() WHERE id = 'ws_race'")
        .execute(&mut *deleter)
        .await
        .unwrap();
    let grant =
        memberships.upsert_workspace_member(workspace_member("org_ws_race", "ws_race", "owner"));
    tokio::pin!(grant);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), &mut grant)
            .await
            .is_err(),
        "the grant must wait for the uncommitted workspace delete"
    );
    deleter.commit().await.unwrap();

    assert!(matches!(
        grant.await,
        Err(PortStorageError::NotFound { .. })
    ));
    assert_eq!(workspace_grant_count(&pool, "ws_race").await, 0);
}

/// A workspace grant serializes with a concurrent removal of the principal's
/// org grant: while the removal holds the org row the grant waits, and once
/// it commits the grant is refused — never left without its org grant.
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_workspace_grant_waits_for_a_concurrent_org_grant_removal() {
    let backend = PostgresBackend::default();
    let pool = backend.pool().await;
    seed_workspace(&backend, "org_grant_race", "ws_grant_race", "owner").await;
    let memberships = backend.membership_store().await;
    memberships
        .upsert_org_member_guarded(org_member(
            "org_grant_race",
            "target",
            OrgMembershipRole::Member,
        ))
        .await
        .unwrap();

    // The guarded removal's critical section: org row lock, then delete.
    let mut remover = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM orgs WHERE id = 'org_grant_race' FOR UPDATE")
        .execute(&mut *remover)
        .await
        .unwrap();
    sqlx::query(
        "DELETE FROM org_memberships WHERE org_id = 'org_grant_race' AND principal_id = 'target'",
    )
    .execute(&mut *remover)
    .await
    .unwrap();
    let grant = memberships.upsert_workspace_member(workspace_member(
        "org_grant_race",
        "ws_grant_race",
        "target",
    ));
    tokio::pin!(grant);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), &mut grant)
            .await
            .is_err(),
        "the grant must wait for the uncommitted org-grant removal"
    );
    remover.commit().await.unwrap();

    assert!(matches!(
        grant.await,
        Err(PortStorageError::NotFound { .. })
    ));
    assert_eq!(workspace_grant_count(&pool, "ws_grant_race").await, 0);
}

/// Roles and principal kinds are closed vocabularies in the schema: a row
/// outside them cannot be written, whatever the writer.
#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_membership_vocabularies_are_closed_by_the_schema() {
    let backend = PostgresBackend::default();
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
