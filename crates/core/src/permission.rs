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
}
