use std::fmt;

use serde::{Deserialize, Serialize};

/// Organization-level role. Higher discriminant = more privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[non_exhaustive]
pub enum OrgRole {
    OrgMember = 0,
    OrgBilling = 1,
    OrgAdmin = 2,
    OrgOwner = 3,
}

/// Workspace-level role. Higher discriminant = more privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[non_exhaustive]
pub enum WorkspaceRole {
    WorkspaceViewer = 0,
    WorkspaceRunner = 1,
    WorkspaceEditor = 2,
    WorkspaceAdmin = 3,
}

impl OrgRole {
    /// OrgAdmin and OrgOwner automatically have WorkspaceAdmin in all workspaces.
    #[must_use]
    pub fn implied_workspace_role(self) -> Option<WorkspaceRole> {
        match self {
            Self::OrgAdmin | Self::OrgOwner => Some(WorkspaceRole::WorkspaceAdmin),
            Self::OrgMember | Self::OrgBilling => None,
        }
    }
}

/// Compute the effective workspace role, considering org-level implication.
#[must_use]
pub fn effective_workspace_role(
    org_role: Option<OrgRole>,
    explicit_ws_role: Option<WorkspaceRole>,
) -> Option<WorkspaceRole> {
    let implied = org_role.and_then(OrgRole::implied_workspace_role);
    match (implied, explicit_ws_role) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

impl fmt::Display for OrgRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrgMember => write!(f, "OrgMember"),
            Self::OrgBilling => write!(f, "OrgBilling"),
            Self::OrgAdmin => write!(f, "OrgAdmin"),
            Self::OrgOwner => write!(f, "OrgOwner"),
        }
    }
}

impl fmt::Display for WorkspaceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkspaceViewer => write!(f, "WorkspaceViewer"),
            Self::WorkspaceRunner => write!(f, "WorkspaceRunner"),
            Self::WorkspaceEditor => write!(f, "WorkspaceEditor"),
            Self::WorkspaceAdmin => write!(f, "WorkspaceAdmin"),
        }
    }
}
