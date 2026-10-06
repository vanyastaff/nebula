// Backend trait, row builders and contract assertions shared by
// `identity_conformance` (in-memory + SQLite) and `identity_conformance_postgres`.
// Textually `include!`d at each binary's crate root.

use std::sync::Arc;

use nebula_storage_port::dto::{
    OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembershipRole, OrgRow,
    PrincipalKind, ResourceRow, ScopeKind, TenantDefaultWorkspaceCreate, TenantMembershipSnapshot,
    TenantOrgCreate, TenantProvisioningConflict, TenantProvisioningOutcome,
    TenantProvisioningRequest, TriggerRow, WorkspaceMemberUpsert, WorkspaceMembershipRole,
    WorkspaceRow,
};
use nebula_storage_port::store::{
    MembershipStore, OrgStore, ResourceStore, TenantProvisioningStore, TriggerStore,
    WorkspaceStore,
};
use nebula_storage_port::{Scope, StorageError as PortStorageError};

/// A storage backend under identity conformance test.

#[async_trait::async_trait]
trait IdentityBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn org_store(&self) -> Arc<dyn OrgStore>;
    async fn workspace_store(&self) -> Arc<dyn WorkspaceStore>;
    async fn membership_store(&self) -> Arc<dyn MembershipStore>;
    async fn tenant_provisioning_store(&self) -> Arc<dyn TenantProvisioningStore>;
    async fn resource_store(&self) -> Arc<dyn ResourceStore>;
    async fn trigger_store(&self) -> Arc<dyn TriggerStore>;
}


// ── row builders ──────────────────────────────────────────────────────────

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
        topology: Some(serde_json::json!({ "max_size": 4 })),
        resilience_override: Some(serde_json::json!({ "requests": 10, "period_ms": 1000 })),
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

// ── shared contract assertions ────────────────────────────────────────────

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

    workspaces
        .create(workspace_row("ws_b", "org_b", "b"))
        .await
        .unwrap();
    let mut service_grant = workspace_member("org_b", "ws_b", "same");
    service_grant.principal_kind = PrincipalKind::ServiceAccount;
    s.upsert_workspace_member(service_grant.clone())
        .await
        .unwrap();
    orgs.soft_delete("org_b").await.unwrap();
    assert!(matches!(
        s.upsert_workspace_member(service_grant).await,
        Err(nebula_storage_port::StorageError::NotFound { .. })
    ));
    assert!(
        !s.remove_workspace_member("org_b", "ws_b", PrincipalKind::ServiceAccount, "same")
            .await
            .unwrap()
    );
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
        Err(PortStorageError::Corrupt(_))
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

