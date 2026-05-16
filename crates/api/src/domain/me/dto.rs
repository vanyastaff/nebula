//! "Me" endpoint DTOs — user profile, organisations, personal access tokens.
//!
//! The profile + PAT DTOs back **live** endpoints (`GET`/`PATCH /me`,
//! `GET`/`POST /me/tokens`, `DELETE /me/tokens/{pat}`) served end-to-end
//! through the Plane-A `AuthBackend` port. [`MyOrgsResponse`] still backs a
//! single honest 501 stub (`GET /me/orgs`): principal→orgs enumeration is
//! not wired end-to-end until the org/membership phase (canon §4.5).
//! Consequently [`MeResponse::orgs_count`] is `Option<u32>` and omitted
//! from the wire (never a synthesized `0`) until that enumeration lands.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// `OrgRoleDto` / `WorkspaceRoleDto` are cross-domain API-boundary role
// wrappers (ADR-0047 §Wrapping-checklist) and live in `domain::shared`
// alongside the other cross-cutting wire DTOs; imported here for the
// `OrgSummary` field below.
use crate::domain::shared::OrgRoleDto;

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
    ///
    /// **Absent** until principal→orgs membership enumeration is wired
    /// end-to-end (lands with the org/membership phase — see
    /// `GET /api/v1/me/orgs`). The field is omitted from the JSON rather
    /// than reported as a synthesized `0`, because a count the system
    /// cannot actually compute would be a false value on the wire
    /// (canon §4.5 / §12.2). When the enumeration lands it starts
    /// appearing as `Some(n)` — additive and non-breaking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orgs_count: Option<u32>,
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
    /// Plaintext token, prefixed `pat_…` (the `PAT_PREFIX` minted by the
    /// auth backend). Shown once; cannot be retrieved later. Treat as a
    /// secret credential.
    #[schema(format = "password", write_only = true)]
    pub token: String,
    /// Metadata for the newly created token.
    pub summary: TokenSummary,
}
