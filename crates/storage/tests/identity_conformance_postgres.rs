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
    async fn user_store(&self) -> Arc<dyn UserStore> {
        Arc::new(nebula_storage::postgres::PgUserStore::new(
            self.pool().await,
        ))
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
    async fn quota_store(&self) -> Arc<dyn QuotaStore> {
        Arc::new(nebula_storage::postgres::PgQuotaStore::new(
            self.pool().await,
        ))
    }
    async fn audit_store(&self) -> Arc<dyn AuditStore> {
        Arc::new(nebula_storage::postgres::PgAuditStore::new(
            self.pool().await,
        ))
    }
    async fn blob_store(&self) -> Arc<dyn BlobStore> {
        Arc::new(nebula_storage::postgres::PgBlobStore::new(
            self.pool().await,
        ))
    }
}

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
