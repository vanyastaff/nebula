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
    pub workspace_role: Option<WorkspaceGrant>,
}

/// A workspace role bound to the workspace it was resolved from.
///
/// Keeping the workspace identifier next to the role prevents a caller from
/// accidentally reusing a role from workspace A while authorizing workspace B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceGrant {
    pub workspace_id: WorkspaceId,
    pub role: WorkspaceRole,
}

impl WorkspaceGrant {
    /// Construct a workspace-scoped role grant.
    #[must_use]
    pub const fn new(workspace_id: WorkspaceId, role: WorkspaceRole) -> Self {
        Self { workspace_id, role }
    }
}

impl TenantContext {
    /// Check that the caller has the given permission.
    /// Returns `Ok(())` if permitted, or an error description if not.
    pub fn require(&self, permission: Permission) -> Result<(), PermissionDenied> {
        if let Some(required_ws_role) = permission.required_workspace_role() {
            match (self.workspace_id, self.workspace_role) {
                (Some(context_workspace), Some(grant))
                    if grant.workspace_id == context_workspace
                        && grant.role >= required_ws_role =>
                {
                    Ok(())
                },
                (Some(context_workspace), Some(grant))
                    if grant.workspace_id == context_workspace =>
                {
                    Err(PermissionDenied {
                        permission,
                        denial: PermissionDenial::Workspace {
                            required: required_ws_role,
                            current: Some(grant.role),
                        },
                    })
                },
                (context_workspace, Some(grant)) => Err(PermissionDenied {
                    permission,
                    denial: PermissionDenial::WorkspaceScopeMismatch {
                        context: context_workspace,
                        grant: grant.workspace_id,
                    },
                }),
                (_, None) => Err(PermissionDenied {
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
    /// A workspace role was present but belonged to a different workspace than the
    /// request context.
    WorkspaceScopeMismatch {
        /// The workspace being authorized, or `None` when no workspace context exists.
        context: Option<WorkspaceId>,
        /// The workspace from which the role grant was resolved.
        grant: WorkspaceId,
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
                write!(
                    f,
                    "workspace role {required} required, current {current_display}"
                )
            },
            Self::WorkspaceScopeMismatch { context, grant } => {
                let context_display = context
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".to_owned());
                write!(
                    f,
                    "workspace role grant belongs to workspace {grant}, current context {context_display}"
                )
            },
            Self::Org { required, current } => {
                let current_display = current
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "none".to_owned());
                write!(f, "org role {required} required, current {current_display}")
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
///
/// This struct is `#[non_exhaustive]`: additional resolved IDs may be added
/// in future versions. External struct literals must use `..Default::default()`.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
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

    fn context_with_workspace_grant(
        workspace_id: WorkspaceId,
        workspace_role: WorkspaceRole,
    ) -> TenantContext {
        TenantContext {
            org_id: OrgId::new(),
            workspace_id: Some(workspace_id),
            principal: Principal::User(UserId::new()),
            org_role: Some(OrgRole::OrgMember),
            workspace_role: Some(WorkspaceGrant::new(workspace_id, workspace_role)),
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

    #[test]
    fn permission_denial_display_discriminates_workspace_vs_org() {
        use crate::role::{OrgRole, WorkspaceRole};

        let ws_denial = PermissionDenial::Workspace {
            required: WorkspaceRole::WorkspaceViewer,
            current: None,
        };
        let org_denial = PermissionDenial::Org {
            required: OrgRole::OrgAdmin,
            current: Some(OrgRole::OrgMember),
        };

        let ws_str = ws_denial.to_string();
        let org_str = org_denial.to_string();

        assert!(
            ws_str.contains("workspace"),
            "workspace denial Display must contain 'workspace', got: {ws_str:?}"
        );
        assert!(
            org_str.contains("org"),
            "org denial Display must contain 'org', got: {org_str:?}"
        );
        assert_ne!(
            ws_str, org_str,
            "workspace and org denial Display strings must differ"
        );
    }

    #[test]
    fn workspace_permission_requires_grant_for_same_workspace() {
        let workspace_a = WorkspaceId::new();
        let workspace_b = WorkspaceId::new();
        let ctx = TenantContext {
            org_id: OrgId::new(),
            workspace_id: Some(workspace_b),
            principal: Principal::User(UserId::new()),
            org_role: Some(OrgRole::OrgMember),
            workspace_role: Some(WorkspaceGrant::new(
                workspace_a,
                WorkspaceRole::WorkspaceAdmin,
            )),
        };

        let err = ctx
            .require(Permission::WorkflowDelete)
            .expect_err("workspace A grant must not authorize workspace B");
        assert_eq!(
            err.denial,
            PermissionDenial::WorkspaceScopeMismatch {
                context: Some(workspace_b),
                grant: workspace_a,
            }
        );
    }

    #[test]
    fn workspace_permission_allows_matching_workspace_grant() {
        let workspace_id = WorkspaceId::new();
        let ctx = context_with_workspace_grant(workspace_id, WorkspaceRole::WorkspaceAdmin);

        assert!(ctx.require(Permission::WorkflowDelete).is_ok());
    }
}
