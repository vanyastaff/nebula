//! API adapter for the storage-owned tenant directory.
//!
//! The API and storage ports deliberately use their own role and identity
//! types. This adapter is the composition boundary: it translates the closed
//! vocabularies, redacts backend failures, and keeps membership reads and
//! writes on the same storage authority as organization/workspace lookup.

use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use nebula_api::{
    ApiError,
    state::{
        AddMemberOutcome, MembershipStore as ApiMembershipStore, OrgMember, OrgResolver,
        RemoveMemberOutcome, TenantMembershipSnapshot, WorkspaceResolver,
    },
};
use nebula_core::{
    OrgId, OrgRole, ServiceAccountId, UserId, WorkspaceId, WorkspaceRole, scope::Principal,
};
use nebula_storage_port::{
    StorageError,
    dto::{
        OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembershipRole,
        PrincipalKind, ScopeKind, WorkspaceMembershipRole,
    },
    store::{MembershipStore, OrgStore, TenantProvisioningStore, WorkspaceStore},
};

/// Storage projections that form one tenant-directory authority.
///
/// Constructors accept one shared backend handle, so callers cannot compose
/// membership and directory reads from unrelated databases.
pub(crate) struct TenantDirectoryStores {
    memberships: Arc<dyn MembershipStore>,
    orgs: Arc<dyn OrgStore>,
    workspaces: Arc<dyn WorkspaceStore>,
    provisioner: Arc<dyn TenantProvisioningStore>,
}

impl TenantDirectoryStores {
    pub(crate) fn memory(directory: &nebula_storage::inmem::InMemoryIdentityDirectory) -> Self {
        Self {
            memberships: Arc::new(directory.membership_store()),
            orgs: Arc::new(directory.org_store()),
            workspaces: Arc::new(directory.workspace_store()),
            provisioner: Arc::new(directory.clone()),
        }
    }

    pub(crate) fn sqlite(pool: sqlx::SqlitePool) -> Self {
        Self {
            memberships: Arc::new(nebula_storage::sqlite::SqliteMembershipStore::new(
                pool.clone(),
            )),
            orgs: Arc::new(nebula_storage::sqlite::SqliteOrgStore::new(pool.clone())),
            workspaces: Arc::new(nebula_storage::sqlite::SqliteWorkspaceStore::new(
                pool.clone(),
            )),
            provisioner: Arc::new(nebula_storage::sqlite::SqliteTenantProvisioningStore::new(
                pool,
            )),
        }
    }

    #[cfg(feature = "postgres")]
    pub(crate) fn postgres(pool: sqlx::PgPool) -> Self {
        Self {
            memberships: Arc::new(nebula_storage::postgres::PgMembershipStore::new(
                pool.clone(),
            )),
            orgs: Arc::new(nebula_storage::postgres::PgOrgStore::new(pool.clone())),
            workspaces: Arc::new(nebula_storage::postgres::PgWorkspaceStore::new(
                pool.clone(),
            )),
            provisioner: Arc::new(nebula_storage::postgres::PgTenantProvisioningStore::new(
                pool,
            )),
        }
    }

    pub(crate) fn provisioner(&self) -> Arc<dyn TenantProvisioningStore> {
        Arc::clone(&self.provisioner)
    }
}

/// One API-facing view over the storage-owned tenant directory.
///
/// Composition must supply projections backed by the same database or shared
/// in-memory directory. Keeping them together here prevents API call sites
/// from accidentally mixing identity authorities.
#[derive(Clone)]
pub(crate) struct ServerTenantDirectory {
    memberships: Arc<dyn MembershipStore>,
    orgs: Arc<dyn OrgStore>,
    workspaces: Arc<dyn WorkspaceStore>,
}

impl std::fmt::Debug for ServerTenantDirectory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerTenantDirectory")
            .finish_non_exhaustive()
    }
}

impl ServerTenantDirectory {
    pub(crate) fn new(stores: TenantDirectoryStores) -> Self {
        Self {
            memberships: stores.memberships,
            orgs: stores.orgs,
            workspaces: stores.workspaces,
        }
    }
}

fn unavailable(operation: &'static str, error: &StorageError) -> ApiError {
    let category = match error {
        StorageError::NotFound { .. } => "not_found",
        StorageError::Conflict { .. } => "conflict",
        StorageError::Duplicate { .. } => "duplicate",
        StorageError::LeaseUnavailable { .. } => "lease_unavailable",
        StorageError::FencedOut { .. } => "fenced_out",
        StorageError::Timeout { .. } => "timeout",
        StorageError::UnknownSchemaVersion { .. } => "unknown_schema_version",
        StorageError::ScopeViolation { .. } => "scope_violation",
        StorageError::Serialization(_) => "serialization",
        StorageError::Connection(_) => "connection",
        StorageError::AcknowledgementUnknown { .. } => "acknowledgement_unknown",
        StorageError::Configuration(_) => "configuration",
        StorageError::Internal(_) => "internal",
        _ => "unknown",
    };
    tracing::error!(
        operation,
        error.category = category,
        "tenant directory operation failed"
    );
    ApiError::ServiceUnavailable("Tenant directory is unavailable".to_owned())
}

fn corrupt(operation: &'static str) -> ApiError {
    tracing::error!(
        operation,
        "tenant directory returned malformed identity data"
    );
    ApiError::ServiceUnavailable("Tenant directory is unavailable".to_owned())
}

fn principal_parts(principal: &Principal) -> Result<(PrincipalKind, String), ApiError> {
    match principal {
        Principal::User(id) => Ok((PrincipalKind::User, id.to_string())),
        Principal::ServiceAccount(id) => Ok((PrincipalKind::ServiceAccount, id.to_string())),
        _ => Err(ApiError::Forbidden(
            "principal cannot hold tenant membership".to_owned(),
        )),
    }
}

const fn org_role_from_storage(role: OrgMembershipRole) -> OrgRole {
    match role {
        OrgMembershipRole::Member => OrgRole::OrgMember,
        OrgMembershipRole::Billing => OrgRole::OrgBilling,
        OrgMembershipRole::Admin => OrgRole::OrgAdmin,
        OrgMembershipRole::Owner => OrgRole::OrgOwner,
    }
}

fn org_role_to_storage(role: OrgRole) -> Result<OrgMembershipRole, ApiError> {
    match role {
        OrgRole::OrgMember => Ok(OrgMembershipRole::Member),
        OrgRole::OrgBilling => Ok(OrgMembershipRole::Billing),
        OrgRole::OrgAdmin => Ok(OrgMembershipRole::Admin),
        OrgRole::OrgOwner => Ok(OrgMembershipRole::Owner),
        _ => Err(ApiError::ServiceUnavailable(
            "Tenant role is unsupported".to_owned(),
        )),
    }
}

const fn workspace_role_from_storage(role: WorkspaceMembershipRole) -> WorkspaceRole {
    match role {
        WorkspaceMembershipRole::Viewer => WorkspaceRole::WorkspaceViewer,
        WorkspaceMembershipRole::Runner => WorkspaceRole::WorkspaceRunner,
        WorkspaceMembershipRole::Editor => WorkspaceRole::WorkspaceEditor,
        WorkspaceMembershipRole::Admin => WorkspaceRole::WorkspaceAdmin,
    }
}

fn stored_principal(kind: PrincipalKind, id: &str) -> Result<Principal, ApiError> {
    match kind {
        PrincipalKind::User => UserId::from_str(id)
            .map(Principal::User)
            .map_err(|_| corrupt("list_members")),
        PrincipalKind::ServiceAccount => ServiceAccountId::from_str(id)
            .map(Principal::ServiceAccount)
            .map_err(|_| corrupt("list_members")),
    }
}

#[async_trait]
impl OrgResolver for ServerTenantDirectory {
    async fn resolve_by_slug(&self, slug: &str) -> Result<OrgId, ApiError> {
        let row = self
            .orgs
            .get_by_slug(slug)
            .await
            .map_err(|error| unavailable("resolve_org_by_slug", &error))?
            .ok_or_else(|| ApiError::NotFound("organization not found".to_owned()))?;
        OrgId::from_str(&row.id).map_err(|_| corrupt("resolve_org_by_slug"))
    }
}

#[async_trait]
impl WorkspaceResolver for ServerTenantDirectory {
    async fn resolve_by_slug(&self, org_id: OrgId, slug: &str) -> Result<WorkspaceId, ApiError> {
        let row = self
            .workspaces
            .get_by_slug(&org_id.to_string(), slug)
            .await
            .map_err(|error| unavailable("resolve_workspace_by_slug", &error))?
            .ok_or_else(|| ApiError::NotFound("workspace not found".to_owned()))?;
        WorkspaceId::from_str(&row.id).map_err(|_| corrupt("resolve_workspace_by_slug"))
    }

    async fn resolve_by_id(
        &self,
        org_id: OrgId,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceId, ApiError> {
        let found = self
            .workspaces
            .get(&org_id.to_string(), &workspace_id.to_string())
            .await
            .map_err(|error| unavailable("resolve_workspace_by_id", &error))?;
        found
            .map(|_| workspace_id)
            .ok_or_else(|| ApiError::NotFound("workspace not found".to_owned()))
    }
}

#[async_trait]
impl ApiMembershipStore for ServerTenantDirectory {
    async fn get_tenant_membership(
        &self,
        org_id: OrgId,
        workspace_id: Option<WorkspaceId>,
        principal: &Principal,
    ) -> Result<TenantMembershipSnapshot, ApiError> {
        let (kind, id) = principal_parts(principal)?;
        let snapshot = self
            .memberships
            .get_tenant_membership(
                &org_id.to_string(),
                workspace_id.as_ref().map(ToString::to_string).as_deref(),
                kind,
                &id,
            )
            .await
            .map_err(|error| unavailable("get_tenant_membership", &error))?;
        Ok(TenantMembershipSnapshot {
            org_role: snapshot.org_role.map(org_role_from_storage),
            workspace_role: snapshot.workspace_role.map(workspace_role_from_storage),
        })
    }

    async fn get_org_role(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<Option<OrgRole>, ApiError> {
        Ok(self
            .get_tenant_membership(org_id, None, principal)
            .await?
            .org_role)
    }

    async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>, ApiError> {
        let rows = self
            .memberships
            .list_for_scope(ScopeKind::Org, &org_id.to_string())
            .await
            .map_err(|error| unavailable("list_members", &error))?;
        rows.into_iter()
            .map(|row| {
                let role = OrgMembershipRole::parse(&row.role)
                    .map(org_role_from_storage)
                    .map_err(|_| corrupt("list_members"))?;
                Ok(OrgMember {
                    principal: stored_principal(row.principal_kind, &row.principal_id)?,
                    role,
                })
            })
            .collect()
    }

    async fn add_member_guarded(
        &self,
        org_id: OrgId,
        principal: &Principal,
        role: OrgRole,
    ) -> Result<AddMemberOutcome, ApiError> {
        let (kind, id) = principal_parts(principal)?;
        let outcome = self
            .memberships
            .upsert_org_member_guarded(OrgMemberUpsert {
                org_id: org_id.to_string(),
                principal_kind: kind,
                principal_id: id,
                role: org_role_to_storage(role)?,
                added_by: None,
            })
            .await
            .map_err(|error| unavailable("add_member_guarded", &error))?;
        Ok(match outcome {
            OrgMemberUpsertOutcome::Applied => AddMemberOutcome::Added,
            OrgMemberUpsertOutcome::WouldLockOut => AddMemberOutcome::WouldLockOut,
        })
    }

    async fn remove_member_guarded(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<RemoveMemberOutcome, ApiError> {
        let (kind, id) = principal_parts(principal)?;
        let outcome = self
            .memberships
            .remove_org_member_guarded(&org_id.to_string(), kind, &id)
            .await
            .map_err(|error| unavailable("remove_member_guarded", &error))?;
        Ok(match outcome {
            OrgMemberRemoveOutcome::Removed => RemoveMemberOutcome::Removed,
            OrgMemberRemoveOutcome::NotFound => RemoveMemberOutcome::NotFound,
            OrgMemberRemoveOutcome::WouldLockOut => RemoveMemberOutcome::WouldLockOut,
        })
    }

    async fn list_orgs_for_principal(
        &self,
        principal: &Principal,
    ) -> Result<Vec<(OrgId, OrgRole)>, ApiError> {
        let (kind, id) = principal_parts(principal)?;
        self.memberships
            .list_orgs_for_principal(kind, &id)
            .await
            .map_err(|error| unavailable("list_orgs_for_principal", &error))?
            .into_iter()
            .map(|row| {
                let org_id =
                    OrgId::from_str(&row.org_id).map_err(|_| corrupt("list_orgs_for_principal"))?;
                Ok((org_id, org_role_from_storage(row.role)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use nebula_api::state::{MembershipStore as _, WorkspaceResolver as _};
    use nebula_core::{OrgId, OrgRole, UserId, WorkspaceId, scope::Principal};
    use nebula_storage::inmem::InMemoryIdentityDirectory;
    use nebula_storage_port::{
        dto::{OrgRow, WorkspaceRow},
        store::{OrgStore as _, WorkspaceStore as _},
    };

    use super::ServerTenantDirectory;

    async fn directory() -> (ServerTenantDirectory, OrgId, WorkspaceId, Principal) {
        let stores = InMemoryIdentityDirectory::new();
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();
        let owner = UserId::new();
        stores
            .org_store()
            .create(OrgRow {
                id: org_id.to_string(),
                slug: "acme".to_owned(),
                display_name: "Acme".to_owned(),
                created_at: "2026-09-22T00:00:00Z".to_owned(),
                created_by: owner.to_string(),
                plan: "team".to_owned(),
                billing_email: None,
                settings: serde_json::json!({}),
                version: 1,
                deleted_at: None,
            })
            .await
            .expect("org fixture is valid");
        stores
            .workspace_store()
            .create(WorkspaceRow {
                id: workspace_id.to_string(),
                org_id: org_id.to_string(),
                slug: "main".to_owned(),
                display_name: "Main".to_owned(),
                description: None,
                created_at: "2026-09-22T00:00:00Z".to_owned(),
                created_by: owner.to_string(),
                is_default: true,
                settings: serde_json::json!({}),
                version: 1,
                deleted_at: None,
            })
            .await
            .expect("workspace fixture is valid");
        let adapter = ServerTenantDirectory::new(super::TenantDirectoryStores::memory(&stores));
        (adapter, org_id, workspace_id, Principal::User(owner))
    }

    #[tokio::test]
    async fn shared_directory_resolves_rows_and_authorizes_inserted_owner() {
        let (directory, org_id, workspace_id, owner) = directory().await;

        assert_eq!(
            nebula_api::state::OrgResolver::resolve_by_slug(&directory, "acme")
                .await
                .expect("org resolves"),
            org_id
        );
        assert_eq!(
            nebula_api::state::WorkspaceResolver::resolve_by_slug(&directory, org_id, "main")
                .await
                .expect("workspace resolves"),
            workspace_id
        );
        let empty = directory
            .get_tenant_membership(org_id, Some(workspace_id), &owner)
            .await
            .expect("empty directory remains readable");
        assert_eq!(empty.org_role, None);
        assert_eq!(empty.workspace_role, None);
        assert_eq!(
            directory
                .add_member_guarded(org_id, &owner, OrgRole::OrgOwner)
                .await
                .expect("owner is inserted"),
            nebula_api::state::AddMemberOutcome::Added
        );
        let snapshot = directory
            .get_tenant_membership(org_id, Some(workspace_id), &owner)
            .await
            .expect("membership snapshot loads");
        assert_eq!(snapshot.org_role, Some(OrgRole::OrgOwner));
        assert_eq!(snapshot.workspace_role, None);
    }

    #[tokio::test]
    async fn removing_only_privileged_member_preserves_lockout_invariant() {
        let (directory, org_id, _, owner) = directory().await;
        directory
            .add_member_guarded(org_id, &owner, OrgRole::OrgOwner)
            .await
            .expect("owner is inserted");

        assert_eq!(
            directory
                .remove_member_guarded(org_id, &owner)
                .await
                .expect("guarded removal has a typed outcome"),
            nebula_api::state::RemoveMemberOutcome::WouldLockOut
        );
    }

    #[tokio::test]
    async fn workspace_lookup_is_scoped_to_parent_org() {
        let (directory, _, workspace_id, _) = directory().await;
        let other_org = OrgId::new();

        let error = directory
            .resolve_by_id(other_org, workspace_id)
            .await
            .expect_err("cross-parent lookup is hidden");
        assert!(matches!(error, nebula_api::ApiError::NotFound(_)));

        let error = directory
            .resolve_by_slug(other_org, "main")
            .await
            .expect_err("cross-parent slug lookup is hidden");
        assert!(matches!(error, nebula_api::ApiError::NotFound(_)));
    }
}
