//! "Me" endpoint DTOs — user profile, organisations, personal access tokens.
//!
//! The profile + PAT DTOs back **live** endpoints (`GET`/`PATCH /me`,
//! `GET`/`POST /me/tokens`, `DELETE /me/tokens/{pat}`) served end-to-end
//! through the Plane-A `AuthBackend` port. [`MyOrgsResponse`] now also
//! backs a **live** endpoint (`GET /me/orgs`): principal→orgs enumeration
//! is wired end-to-end via the shared
//! [`MembershipStore`](crate::state::MembershipStore) (Phase 3). The
//! Phase-2 carry-over is resolved: [`MeResponse::orgs_count`] is now the
//! **real** member count (`Some(n)`), no longer omitted.

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
    /// The **real** member count, derived from the shared
    /// [`MembershipStore`](crate::state::MembershipStore) principal→orgs
    /// enumeration (Phase 3 — resolves the Phase-2 carry-over where this
    /// was omitted because no enumeration existed). Still `Option<u32>`
    /// for forward-compatibility and so it degrades to *absent* (never a
    /// synthesized `0`) if the membership store is unwired — canon §4.5.
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
///
/// `slug` is intentionally absent: the membership store keys by `OrgId`
/// and there is no `OrgId`→slug reverse directory (the `OrgResolver` port
/// is one-way slug→id). Echoing a synthesized slug would be a canon §4.5
/// false field — same reasoning as the dropped `MemberSummary.email`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgSummary {
    /// `org_<ULID>` identifier.
    pub id: String,
    /// Caller's role within this organisation (canonical wire token:
    /// `member` / `billing` / `admin` / `owner`).
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
