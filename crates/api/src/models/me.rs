//! "Me" endpoint DTOs — user profile, organisations, personal access tokens.
//!
//! All endpoints under `/api/v1/me/*` currently return 501 (see audit, class
//! (c)); the DTOs below describe the **planned** payload shape per ADR-0047
//! Stub Endpoint Policy. Once the underlying milestone closes, the only diff
//! is removing `deprecated = true` and the 501 response from each
//! `#[utoipa::path]` annotation.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Wrapper around `nebula_core::OrgRole` exposed at the API boundary.
///
/// Per ADR-0047 cross-layer schema strategy, the API contract MUST NOT embed
/// `nebula_core` types directly in OpenAPI components. The string form is
/// stable across role-set evolutions in the core crate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "owner")]
pub struct OrgRoleDto(pub String);

/// Wrapper around `nebula_core::WorkspaceRole`.
///
/// Same reasoning as [`OrgRoleDto`] — wrap at the API boundary so
/// workspace-role taxonomy changes in `nebula-core` don't ripple into the
/// public spec.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "editor")]
pub struct WorkspaceRoleDto(pub String);

/// `GET /api/v1/me` response — authenticated user's own profile snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MeResponse {
    /// `user_<ULID>` identifier of the authenticated user.
    pub user_id: String,
    /// Lowercased email address.
    pub email: String,
    /// Caller-chosen display name.
    pub display_name: String,
    /// `true` once the user has confirmed their email.
    pub email_verified: bool,
    /// `true` when TOTP is enrolled.
    pub mfa_enabled: bool,
    /// Number of organisations this user belongs to.
    pub orgs_count: u32,
    /// Number of personal access tokens issued to this user.
    pub tokens_count: u32,
}

/// `PATCH /api/v1/me` request body — partial profile update.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateMeRequest {
    /// Replacement display name. Omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Replacement avatar URL. Omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// Lightweight org membership summary nested in [`MyOrgsResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgSummary {
    /// `org_<ULID>` identifier.
    pub id: String,
    /// URL-safe slug.
    pub slug: String,
    /// Caller's role within this organisation.
    pub role: OrgRoleDto,
}

/// `GET /api/v1/me/orgs` response — organisations the user is a member of.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MyOrgsResponse {
    /// One entry per org membership, no pagination — assumed bounded per user.
    pub orgs: Vec<OrgSummary>,
}

/// Personal access token summary — **never** includes the token value.
///
/// Returned by [`MyTokensResponse`] and as the `summary` field of
/// [`CreateTokenResponse`]. The actual token string is exposed exactly once,
/// at creation time, via [`CreateTokenResponse::token`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TokenSummary {
    /// `pat_<ULID>` identifier.
    pub id: String,
    /// Caller-chosen friendly name.
    pub name: String,
    /// Granted scopes (e.g. `["workflows:read"]`).
    pub scopes: Vec<String>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 timestamp of last successful authentication, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    /// ISO 8601 expiration timestamp, if the token is bounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// `GET /api/v1/me/tokens` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MyTokensResponse {
    /// Token metadata records — values are never exposed here.
    pub tokens: Vec<TokenSummary>,
}

/// `POST /api/v1/me/tokens` request body.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    /// Caller-chosen friendly name.
    pub name: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// Optional time-to-live in seconds. Omit for non-expiring tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
}

/// `POST /api/v1/me/tokens` response — exposes the token value **once**.
///
/// The `token` field is marked `write_only = true` so OpenAPI consumers
/// understand it cannot be retrieved via subsequent GET calls; only the
/// creation response carries it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateTokenResponse {
    /// Plaintext token, prefixed `nbl_pat_…`. Shown once; cannot be retrieved
    /// later. Treat as a secret credential.
    #[schema(format = "password", write_only = true)]
    pub token: String,
    /// Metadata for the newly created token.
    pub summary: TokenSummary,
}
