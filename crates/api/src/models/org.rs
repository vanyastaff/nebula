//! Organisation-management DTOs.
//!
//! All endpoints under `/api/v1/orgs/{org}/…` currently return 501 (audit
//! class (c)); the DTOs below describe the **planned** payload shape per
//! ADR-0047 Stub Endpoint Policy. RBAC `tenant.require(...)` gates ARE real
//! today — the spec accordingly declares 403 alongside the 501 response.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::me::OrgRoleDto;

/// `GET /api/v1/orgs/{org}` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgResponse {
    /// `org_<ULID>` identifier.
    pub id: String,
    /// URL-safe slug.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Plan tier (e.g. `"free"`, `"team"`, `"enterprise"`).
    pub plan: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// `PATCH /api/v1/orgs/{org}` request body.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateOrgRequest {
    /// Replacement display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Org-level settings blob — caller-defined, validated downstream.
    /// Per ADR-0047 §3 cross-layer schema strategy, an opaque
    /// `serde_json::Value` field documents itself as an object with
    /// `additionalProperties: true` so consumers know the shape is
    /// genuinely caller-defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<serde_json::Value>)]
    pub settings: Option<serde_json::Value>,
}

/// One member entry inside [`MembersResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MemberSummary {
    /// `user_<ULID>` of the member.
    pub user_id: String,
    /// Lowercased email address.
    pub email: String,
    /// Member's role within this organisation.
    pub role: OrgRoleDto,
    /// ISO 8601 timestamp of when the membership was established.
    pub joined_at: String,
}

/// `GET /api/v1/orgs/{org}/members` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MembersResponse {
    /// Member summaries.
    pub members: Vec<MemberSummary>,
}

/// `POST /api/v1/orgs/{org}/members` request body — invite a new member.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct InviteMemberRequest {
    /// Recipient email address.
    pub email: String,
    /// Role to assign on acceptance.
    pub role: OrgRoleDto,
}

/// `POST /api/v1/orgs/{org}/members` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InviteMemberResponse {
    /// `inv_<ULID>` invitation identifier.
    pub invitation_id: String,
    /// ISO 8601 expiration timestamp for the invitation.
    pub expires_at: String,
}

/// One service-account entry in [`ServiceAccountsResponse`] / response of
/// [`CreateServiceAccountResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServiceAccountSummary {
    /// `sa_<ULID>` identifier.
    pub id: String,
    /// Caller-chosen friendly name.
    pub name: String,
    /// Granted scopes (e.g. `["workflows:run"]`).
    pub scopes: Vec<String>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// `GET /api/v1/orgs/{org}/service-accounts` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServiceAccountsResponse {
    /// Service-account summaries.
    pub accounts: Vec<ServiceAccountSummary>,
}

/// `POST /api/v1/orgs/{org}/service-accounts` request body.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateServiceAccountRequest {
    /// Caller-chosen friendly name.
    pub name: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
}

/// `POST /api/v1/orgs/{org}/service-accounts` response.
///
/// The bearer key is exposed **exactly once** — flagged `write_only = true`
/// so OpenAPI consumers don't expect it on subsequent reads.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateServiceAccountResponse {
    /// Service-account metadata.
    pub account: ServiceAccountSummary,
    /// Plaintext bearer key, prefixed `nbl_sa_…`. Shown once; treat as a
    /// secret credential. Cannot be retrieved later.
    #[schema(format = "password", write_only = true)]
    pub key: String,
}
