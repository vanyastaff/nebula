use serde::{Deserialize, Serialize};

use crate::role::WorkspaceRole;

/// Granular permission that can be checked against a workspace role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

    // Workspace membership
    WorkspaceMemberRead,
    WorkspaceMemberManage,

    // Org-level (checked against OrgRole, not WorkspaceRole)
    OrgRead,
    OrgUpdate,
    OrgDelete,
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
            | Self::ExecutionTerminate => Some(WorkspaceRole::WorkspaceEditor),

            // Admin can manage members
            Self::WorkspaceMemberManage => Some(WorkspaceRole::WorkspaceAdmin),

            // Org-level — not mapped to workspace roles
            Self::OrgRead
            | Self::OrgUpdate
            | Self::OrgDelete
            | Self::MemberInvite
            | Self::MemberRemove
            | Self::ServiceAccountManage => None,
        }
    }
}
