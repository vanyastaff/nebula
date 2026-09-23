//! Closed role vocabulary and atomic membership-operation contracts.

use super::PrincipalKind;

/// A persisted role is unknown or belongs to the other membership scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("membership role is invalid")]
pub struct MembershipRoleParseError;

/// Organization role as stored by membership adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgMembershipRole {
    /// Ordinary organization member.
    Member,
    /// Billing administrator without workspace administration.
    Billing,
    /// Organization administrator.
    Admin,
    /// Organization owner.
    Owner,
}

impl OrgMembershipRole {
    /// Stable persisted spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Member => "OrgMember",
            Self::Billing => "OrgBilling",
            Self::Admin => "OrgAdmin",
            Self::Owner => "OrgOwner",
        }
    }

    /// Decode a persisted role without accepting aliases or another scope.
    ///
    /// # Errors
    /// Returns a payload-free error for an unknown role.
    pub fn parse(value: &str) -> Result<Self, MembershipRoleParseError> {
        match value {
            "OrgMember" => Ok(Self::Member),
            "OrgBilling" => Ok(Self::Billing),
            "OrgAdmin" => Ok(Self::Admin),
            "OrgOwner" => Ok(Self::Owner),
            _ => Err(MembershipRoleParseError),
        }
    }

    /// Whether this role prevents organization administrative lockout.
    #[must_use]
    pub const fn is_privileged(self) -> bool {
        matches!(self, Self::Admin | Self::Owner)
    }
}

/// Explicit workspace role; no organization-role implication is applied here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMembershipRole {
    /// Read-only workspace access.
    Viewer,
    /// Permission to run workflows.
    Runner,
    /// Permission to edit workspace content.
    Editor,
    /// Workspace administration.
    Admin,
}

impl WorkspaceMembershipRole {
    /// Stable persisted spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "WorkspaceViewer",
            Self::Runner => "WorkspaceRunner",
            Self::Editor => "WorkspaceEditor",
            Self::Admin => "WorkspaceAdmin",
        }
    }

    /// Decode a persisted role without accepting aliases or another scope.
    ///
    /// # Errors
    /// Returns a payload-free error for an unknown role.
    pub fn parse(value: &str) -> Result<Self, MembershipRoleParseError> {
        match value {
            "WorkspaceViewer" => Ok(Self::Viewer),
            "WorkspaceRunner" => Ok(Self::Runner),
            "WorkspaceEditor" => Ok(Self::Editor),
            "WorkspaceAdmin" => Ok(Self::Admin),
            _ => Err(MembershipRoleParseError),
        }
    }
}

/// Roles observed for one principal in a single consistent read snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TenantMembershipSnapshot {
    /// Explicit organization membership; absence never implies authority.
    pub org_role: Option<OrgMembershipRole>,
    /// Explicit workspace membership, absent when no workspace was requested.
    pub workspace_role: Option<WorkspaceMembershipRole>,
}

/// One organization membership belonging to the requested principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalOrgMembership {
    /// Organization identifier.
    pub org_id: String,
    /// Explicit organization role.
    pub role: OrgMembershipRole,
}

/// One explicit membership belonging to a parent-qualified workspace.
///
/// The workspace identity is supplied to the listing operation, so the row
/// carries only the principal and its closed role vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMembership {
    /// Principal domain.
    pub principal_kind: PrincipalKind,
    /// Principal identifier.
    pub principal_id: String,
    /// Explicit workspace role.
    pub role: WorkspaceMembershipRole,
}

/// Organization membership replacement subject to the lockout invariant.
/// The adapter authors the write timestamp inside its atomic operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgMemberUpsert {
    /// Organization identifier.
    pub org_id: String,
    /// Principal domain.
    pub principal_kind: PrincipalKind,
    /// Principal identifier.
    pub principal_id: String,
    /// New organization role.
    pub role: OrgMembershipRole,
    /// Actor identifier recorded for this write.
    pub added_by: Option<String>,
}

/// Explicit workspace membership replacement; cannot write organization roles.
/// The adapter authors the write timestamp inside its atomic operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMemberUpsert {
    /// Parent organization identifier.
    pub org_id: String,
    /// Workspace identifier, verified against the parent organization.
    pub workspace_id: String,
    /// Principal domain.
    pub principal_kind: PrincipalKind,
    /// Principal identifier.
    pub principal_id: String,
    /// Explicit workspace role.
    pub role: WorkspaceMembershipRole,
    /// Actor identifier recorded for this write.
    pub added_by: Option<String>,
}

/// Outcome of an atomic guarded organization membership upsert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgMemberUpsertOutcome {
    /// The membership was inserted or replaced.
    Applied,
    /// The proposed state would have no owner or administrator.
    WouldLockOut,
}

/// Outcome of an atomic guarded organization membership removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgMemberRemoveOutcome {
    /// The membership was removed.
    Removed,
    /// No membership existed for the specified principal and organization.
    NotFound,
    /// The proposed removal would remove the last owner or administrator.
    WouldLockOut,
}
