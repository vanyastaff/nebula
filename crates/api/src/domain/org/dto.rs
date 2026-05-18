//! Organisation-management DTOs.
//!
//! ## honest capability status
//!
//! The **member** DTOs ([`MemberSummary`], [`MembersResponse`],
//! [`AddMemberRequest`]) back **live** endpoints (`GET`/`POST`/`DELETE`
//! under `…/orgs/{org}/members`), served end-to-end through the shared
//! [`MembershipStore`](crate::state::MembershipStore).
//!
//! The **org-record** DTOs ([`OrgResponse`], [`UpdateOrgRequest`]) and the
//! **service-account** DTOs ([`ServiceAccountSummary`],
//! [`CreateServiceAccountRequest`], …) still describe the **planned**
//! payload of honest-501 stubs (no org-record store; no end-to-end
//! `Principal::ServiceAccount` auth path — honest capability contract). RBAC
//! `tenant.require(...)` gates on those stubs are real today (403).
//!
//! ### Member contract — "Option 1" honest redesign (breaking)
//!
//! `POST /orgs/{org}/members` is **direct add-by-principal-id**, not an
//! email invitation. The fabricated `email` / `invitation_id` /
//! `expires_at` fields were removed: there is no invitation/email
//! subsystem and no email→principal directory, so those fields could only
//! ever be synthesized — exactly the honest capability contract false capability this
//! refactor rejects. `MemberSummary` likewise carries only what the RBAC
//! role index actually knows (`user_id` + `role`); `email`/`joined_at`
//! would require a member-record/identity-join model that does not exist.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::shared::OrgRoleDto;

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
    /// Per 3 cross-layer schema strategy, an opaque
    /// `serde_json::Value` field documents itself as an object with
    /// `additionalProperties: true` so consumers know the shape is
    /// genuinely caller-defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<serde_json::Value>)]
    pub settings: Option<serde_json::Value>,
}

/// One member entry inside [`MembersResponse`].
///
/// Carries **only** what the RBAC role index knows. `email`/`joined_at`
/// are intentionally absent (honest capability contract — see the module docs): the
/// membership store is not a user-identity directory and synthesizing
/// those fields would be a false capability.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MemberSummary {
    /// Stable principal identity of the member — `usr_<ULID>` for a user
    /// or `svc_<ULID>` for a service account. This is the exact string
    /// accepted by `DELETE /orgs/{org}/members/{principal}`.
    pub principal_id: String,
    /// Member's role within this organisation (canonical wire token:
    /// `member` / `billing` / `admin` / `owner`).
    pub role: OrgRoleDto,
}

/// `GET /api/v1/orgs/{org}/members` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MembersResponse {
    /// Member summaries. Unordered; not paginated (membership sets are
    /// bounded per org).
    pub members: Vec<MemberSummary>,
}

/// `POST /api/v1/orgs/{org}/members` request body — **direct
/// add-by-principal-id** (not an email invitation; see the module docs
/// for why the invitation contract was removed).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddMemberRequest {
    /// Principal to add — `usr_<ULID>` (user) or `svc_<ULID>` (service
    /// account). Must parse to a concrete principal; an unparsable value
    /// is a 400.
    pub principal_id: String,
    /// Role to grant. Canonical wire token (`member` / `billing` /
    /// `admin` / `owner`). The granted role is clamped to the caller's
    /// own role — a caller cannot grant a role above their own.
    pub role: OrgRoleDto,
}

/// One service-account entry in [`ServiceAccountsResponse`] / response of
/// [`CreateServiceAccountResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServiceAccountSummary {
    /// `svc_<ULID>` identifier (matches `nebula_core::ServiceAccountId`, prefix `svc_`).
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
