//! API-owned workspace membership wire types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::shared::WorkspaceRoleDto;

/// One explicit workspace grant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceMemberSummary {
    /// Stable user or service-account identity.
    pub principal_id: String,
    /// Explicit workspace role; organization-role implication is not emitted.
    pub role: WorkspaceRoleDto,
}

/// Bounded, unpaginated workspace membership collection.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceMembersResponse {
    /// Explicit grants ordered by stable principal identity.
    pub members: Vec<WorkspaceMemberSummary>,
}

/// Add or replace one explicit workspace grant.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpsertWorkspaceMemberRequest {
    /// Canonical role token: `viewer`, `runner`, `editor`, or `admin`.
    pub role: WorkspaceRoleDto,
}
