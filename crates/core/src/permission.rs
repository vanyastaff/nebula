use serde::{Deserialize, Serialize};

use crate::role::WorkspaceRole;

/// Granular permission that can be checked against a workspace role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Permission {
    // Workflow
    WorkflowRead,
    WorkflowWrite,
    WorkflowDelete,
    WorkflowExecute,

    // Execution
    ExecutionRead,
    ExecutionCancel,
    ExecutionTerminate,
    ExecutionRestart,

    // Credential
    CredentialRead,
    CredentialWrite,
    CredentialDelete,

    // Resource
    ResourceRead,
    ResourceWrite,
    ResourceDelete,

    // Workspace membership
    WorkspaceMemberRead,
    WorkspaceMemberManage,

    // Org-level (checked against OrgRole, not WorkspaceRole)
    OrgRead,
    OrgUpdate,
    OrgDelete,
    MemberRead,
    MemberInvite,
    MemberRemove,
    ServiceAccountManage,

    /// Resolve an ambiguous provider outcome on a poisoned refresh claim.
    ///
    /// Deliberately not a member of the `CredentialWrite` tier, but not
    /// because that tier bounds the damage: an editor already holds
    /// `CredentialDelete` and can revoke provider material. What the admin
    /// tier matches is the decision's nature — an operator judgment about
    /// out-of-band evidence the platform could not observe. Its blast radius
    /// is the smaller one, since a wrong decision surfaces as a provider-side
    /// failure on the next use rather than as a replay authorization.
    ///
    /// Appended rather than grouped with the credential variants above: this
    /// enum derives `Ord` and is `#[non_exhaustive]`, so a variant inserted
    /// mid-enum renumbers and reorders every later one in a consuming build.
    CredentialReconcile,
}

impl Permission {
    /// Minimum workspace role required for this permission.
    /// Returns `None` for org-level permissions that don't map to workspace roles.
    #[must_use]
    pub fn required_workspace_role(self) -> Option<WorkspaceRole> {
        match self {
            // Viewer can read
            Self::WorkflowRead
            | Self::ExecutionRead
            | Self::CredentialRead
            | Self::ResourceRead
            | Self::WorkspaceMemberRead => Some(WorkspaceRole::WorkspaceViewer),

            // Runner can execute
            Self::WorkflowExecute | Self::ExecutionCancel | Self::ExecutionRestart => {
                Some(WorkspaceRole::WorkspaceRunner)
            },

            // Editor can write
            Self::WorkflowWrite
            | Self::WorkflowDelete
            | Self::CredentialWrite
            | Self::CredentialDelete
            | Self::ResourceWrite
            | Self::ResourceDelete
            | Self::ExecutionTerminate => Some(WorkspaceRole::WorkspaceEditor),

            // Admin can manage members
            Self::WorkspaceMemberManage => Some(WorkspaceRole::WorkspaceAdmin),

            // Admin can adjudicate an ambiguous provider outcome: the caller
            // answers for a provider side effect the platform could not
            // observe, on out-of-band evidence, so the tier matches the
            // decision's nature rather than the damage it could do.
            //
            // `TenantContext::require` consults the workspace gate whenever
            // this mapping is `Some`, and an org admin resolves to
            // `WorkspaceAdmin` at the workspace level, so an org role reaches
            // this permission by implication rather than by an org check.
            Self::CredentialReconcile => Some(WorkspaceRole::WorkspaceAdmin),

            // Org-level — not mapped to workspace roles
            Self::OrgRead
            | Self::OrgUpdate
            | Self::OrgDelete
            | Self::MemberRead
            | Self::MemberInvite
            | Self::MemberRemove
            | Self::ServiceAccountManage => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_mutation_permissions_require_workspace_editor() {
        assert_eq!(
            Permission::ResourceRead.required_workspace_role(),
            Some(WorkspaceRole::WorkspaceViewer)
        );
        assert_eq!(
            Permission::ResourceWrite.required_workspace_role(),
            Some(WorkspaceRole::WorkspaceEditor)
        );
        assert_eq!(
            Permission::ResourceDelete.required_workspace_role(),
            Some(WorkspaceRole::WorkspaceEditor)
        );
    }

    #[test]
    fn member_read_is_org_level_permission() {
        assert_eq!(Permission::MemberRead.required_workspace_role(), None);
    }

    #[test]
    fn credential_reconciliation_requires_workspace_admin() {
        // The reconcile seam resolves an unobservable provider outcome on
        // out-of-band evidence, so it is an operator judgment rather than a
        // step in the credential write tier.
        assert_eq!(
            Permission::CredentialReconcile.required_workspace_role(),
            Some(WorkspaceRole::WorkspaceAdmin)
        );
        assert!(
            Permission::CredentialReconcile.required_workspace_role()
                > Permission::CredentialWrite.required_workspace_role()
        );
    }
}
