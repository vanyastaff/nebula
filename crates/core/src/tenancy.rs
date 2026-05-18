use std::fmt;

use crate::{
    id::types::{
        CredentialId, ExecutionId, OrgId, ServiceAccountId, TriggerId, WorkflowId, WorkspaceId,
    },
    permission::Permission,
    role::{OrgRole, WorkspaceRole},
    scope::Principal,
};

/// Tenant context constructed by middleware and passed to handlers.
/// Carries the resolved identifiers and the caller's roles.
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub org_id: OrgId,
    pub workspace_id: Option<WorkspaceId>,
    pub principal: Principal,
    pub org_role: Option<OrgRole>,
    pub workspace_role: Option<WorkspaceRole>,
}

impl TenantContext {
    /// Check that the caller has the given permission.
    /// Returns `Ok(())` if permitted, or an error description if not.
    pub fn require(&self, permission: Permission) -> Result<(), PermissionDenied> {
        if let Some(required_ws_role) = permission.required_workspace_role() {
            // Workspace-level permission
            match self.workspace_role {
                Some(actual) if actual >= required_ws_role => Ok(()),
                Some(actual) => Err(PermissionDenied {
                    permission,
                    required_role: required_ws_role.to_string(),
                    current_role: actual.to_string(),
                }),
                None => Err(PermissionDenied {
                    permission,
                    required_role: required_ws_role.to_string(),
                    current_role: "none".to_string(),
                }),
            }
        } else {
            // Org-level permission — check org_role
            // OrgAdmin+ can do most org ops; OrgOwner for destructive ops
            let required = match permission {
                Permission::OrgRead | Permission::MemberRead => OrgRole::OrgMember,
                Permission::OrgUpdate
                | Permission::MemberInvite
                | Permission::MemberRemove
                | Permission::ServiceAccountManage => OrgRole::OrgAdmin,
                Permission::OrgDelete => OrgRole::OrgOwner,
                Permission::WorkflowRead
                | Permission::WorkflowWrite
                | Permission::WorkflowDelete
                | Permission::WorkflowExecute
                | Permission::ExecutionRead
                | Permission::ExecutionCancel
                | Permission::ExecutionTerminate
                | Permission::ExecutionRestart
                | Permission::CredentialRead
                | Permission::CredentialWrite
                | Permission::CredentialDelete
                | Permission::ResourceRead
                | Permission::ResourceWrite
                | Permission::ResourceDelete
                | Permission::WorkspaceMemberRead
                | Permission::WorkspaceMemberManage => OrgRole::OrgAdmin,
            };
            match self.org_role {
                Some(actual) if actual >= required => Ok(()),
                Some(actual) => Err(PermissionDenied {
                    permission,
                    required_role: required.to_string(),
                    current_role: actual.to_string(),
                }),
                None => Err(PermissionDenied {
                    permission,
                    required_role: required.to_string(),
                    current_role: "none".to_string(),
                }),
            }
        }
    }

    /// Require that a workspace is present in this context.
    pub fn require_workspace(&self) -> Result<WorkspaceId, &'static str> {
        self.workspace_id
            .ok_or("workspace context required but not present")
    }
}

/// Returned when a permission check fails.
#[derive(Debug, Clone)]
pub struct PermissionDenied {
    pub permission: Permission,
    pub required_role: String,
    pub current_role: String,
}

impl fmt::Display for PermissionDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} required, current role {}",
            self.required_role, self.current_role
        )
    }
}

impl std::error::Error for PermissionDenied {}

/// IDs resolved by the tenancy middleware from path segments.
/// Inserted into request extensions so handlers and RBAC middleware can use them.
#[derive(Debug, Clone, Default)]
pub struct ResolvedIds {
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,
    pub workflow_id: Option<WorkflowId>,
    pub execution_id: Option<ExecutionId>,
    pub credential_id: Option<CredentialId>,
    pub trigger_id: Option<TriggerId>,
    pub service_account_id: Option<ServiceAccountId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::UserId;

    fn context_with_org_role(org_role: Option<OrgRole>) -> TenantContext {
        TenantContext {
            org_id: OrgId::new(),
            workspace_id: None,
            principal: Principal::User(UserId::new()),
            org_role,
            workspace_role: None,
        }
    }

    #[test]
    fn org_member_can_read_org_and_members() {
        let ctx = context_with_org_role(Some(OrgRole::OrgMember));

        assert!(ctx.require(Permission::OrgRead).is_ok());
        assert!(ctx.require(Permission::MemberRead).is_ok());
    }

    #[test]
    fn org_member_cannot_manage_org_members_or_service_accounts() {
        let ctx = context_with_org_role(Some(OrgRole::OrgMember));

        for permission in [
            Permission::OrgUpdate,
            Permission::MemberInvite,
            Permission::MemberRemove,
            Permission::ServiceAccountManage,
        ] {
            assert_eq!(
                ctx.require(permission)
                    .map_err(|err| (err.required_role, err.current_role)),
                Err((
                    OrgRole::OrgAdmin.to_string(),
                    OrgRole::OrgMember.to_string()
                ))
            );
        }
    }

    #[test]
    fn only_org_owner_can_delete_org() {
        let admin = context_with_org_role(Some(OrgRole::OrgAdmin));
        let owner = context_with_org_role(Some(OrgRole::OrgOwner));

        assert_eq!(
            admin
                .require(Permission::OrgDelete)
                .map_err(|err| (err.required_role, err.current_role)),
            Err((OrgRole::OrgOwner.to_string(), OrgRole::OrgAdmin.to_string()))
        );
        assert!(owner.require(Permission::OrgDelete).is_ok());
    }
}
