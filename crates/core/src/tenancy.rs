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
                    denial: PermissionDenial::Workspace {
                        required: required_ws_role,
                        current: Some(actual),
                    },
                }),
                None => Err(PermissionDenied {
                    permission,
                    denial: PermissionDenial::Workspace {
                        required: required_ws_role,
                        current: None,
                    },
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
                    denial: PermissionDenial::Org {
                        required,
                        current: Some(actual),
                    },
                }),
                None => Err(PermissionDenied {
                    permission,
                    denial: PermissionDenial::Org {
                        required,
                        current: None,
                    },
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

/// The specific role-level mismatch that caused a permission check to fail.
///
/// Carries the required and actual (caller's) role at the level that was checked.
/// `current` is `None` when the caller has no role at that level at all.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionDenial {
    /// A workspace-scoped permission was required but the caller's workspace role was
    /// absent or below the minimum.
    Workspace {
        /// The minimum workspace role required by the permission.
        required: WorkspaceRole,
        /// The caller's workspace role, or `None` if no workspace role is present.
        current: Option<WorkspaceRole>,
    },
    /// An org-scoped permission was required but the caller's org role was absent or
    /// below the minimum.
    Org {
        /// The minimum org role required by the permission.
        required: OrgRole,
        /// The caller's org role, or `None` if no org role is present.
        current: Option<OrgRole>,
    },
}

impl fmt::Display for PermissionDenial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace { required, current } => {
                let current_display = current
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "none".to_owned());
                write!(f, "{required} required, current role {current_display}")
            },
            Self::Org { required, current } => {
                let current_display = current
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "none".to_owned());
                write!(f, "{required} required, current role {current_display}")
            },
        }
    }
}

/// Returned when a permission check fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDenied {
    /// The permission that was required.
    pub permission: Permission,
    /// The role-level mismatch that caused the denial.
    pub denial: PermissionDenial,
}

impl fmt::Display for PermissionDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.denial, f)
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
            let err = ctx
                .require(permission)
                .expect_err("OrgMember must be denied");
            assert_eq!(
                err.denial,
                PermissionDenial::Org {
                    required: OrgRole::OrgAdmin,
                    current: Some(OrgRole::OrgMember),
                },
                "wrong denial shape for {permission:?}"
            );
        }
    }

    #[test]
    fn only_org_owner_can_delete_org() {
        let admin = context_with_org_role(Some(OrgRole::OrgAdmin));
        let owner = context_with_org_role(Some(OrgRole::OrgOwner));

        let err = admin
            .require(Permission::OrgDelete)
            .expect_err("OrgAdmin must be denied OrgDelete");
        assert_eq!(
            err.denial,
            PermissionDenial::Org {
                required: OrgRole::OrgOwner,
                current: Some(OrgRole::OrgAdmin),
            }
        );
        assert!(owner.require(Permission::OrgDelete).is_ok());
    }
}
