//! User profile endpoint handlers (global, no tenant scope).
//! Auth required but no org/workspace context needed.
//!
//! All six handlers are real end-to-end. Profile read/patch + PAT
//! list/create/revoke delegate to the Plane-A [`AuthBackend`] port;
//! [`list_my_orgs`] (and the `MeResponse.orgs_count` field) delegate to
//! the shared [`MembershipStore`](crate::state::MembershipStore)
//! principal→orgs enumeration (Phase 3 — this resolves the Phase-2
//! carry-over where `list_my_orgs` was an honest 501 and `orgs_count` was
//! omitted because no enumeration backing existed).
//!
//! ## Durability (provisioning durability / engine durability — operator-facing)
//!
//! These endpoints are **implemented and work end-to-end**, but the only
//! wired `AuthBackend` is the in-memory one (`InMemoryAuthBackend`) and
//! the only wired `MembershipStore` is the in-memory one
//! (`InMemoryMembershipStore`). All `me/*` profile, PAT, **and org
//! membership** state is therefore **process-local: it is lost on restart
//! and is NOT shared across replicas.** A PAT minted via `POST /me/tokens`
//! stops authenticating the moment the process exits; an org membership is
//! likewise process-local. This is the same local-first caveat the
//! in-memory idempotency backend carries (see the `crates/api/README.md`
//! idempotency note) — it persists once storage-backed `AuthBackend` /
//! `MembershipStore` adapters land (no such impls exist today;
//! `nebula_storage` ships no `UserRepo`/`PatRepo`/`SessionRepo` and no
//! membership repo). The durability gap is strictly about persistence,
//! not capability.
//!
//! [`AuthBackend`]: crate::domain::auth::backend::AuthBackend

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use nebula_core::Principal;
use zeroize::Zeroize;

use crate::{
    domain::{
        auth::backend::{AuthBackend, CreatePatParams, PatRecord, ProfilePatch},
        me::dto::{
            CreateTokenRequest, CreateTokenResponse, MeResponse, MyOrgsResponse, MyTokensResponse,
            OrgSummary, TokenSummary, UpdateMeRequest,
        },
        shared::{AckResponse, OrgRoleDto},
    },
    error::{ApiError, ApiResult, ProblemDetails},
    middleware::auth::AuthContext,
    state::AppState,
};

/// Extract the caller's `user_<ULID>` identity from the auth context.
///
/// The `me/*` endpoints act strictly on the caller's *own* identity, so
/// only a [`Principal::User`] is a valid subject. API-key (`System`),
/// service-account, and workflow principals have no personal profile or
/// PAT inventory — they are rejected with 401 rather than silently
/// resolving to someone else's data.
fn require_user_id(auth: &AuthContext) -> Result<String, ApiError> {
    match &auth.principal {
        Principal::User(uid) => Ok(uid.to_string()),
        _ => Err(ApiError::Unauthorized(
            "me endpoints require an authenticated user identity".to_owned(),
        )),
    }
}

/// Borrow the wired Plane-A auth backend, or fail closed with 503.
///
/// When the port is unwired the identity surface is genuinely absent;
/// returning 503 (orchestration absent — mirrors the integration seam-step-6 pattern
/// Phase 1 used for the control queue) is the honest degradation, not a
/// fabricated empty success.
fn auth_backend_or_503(state: &AppState) -> Result<&std::sync::Arc<dyn AuthBackend>, ApiError> {
    state.auth_backend.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "authentication backend is not configured; me endpoints are unavailable".to_owned(),
        )
    })
}

/// Map a port-level [`PatRecord`] onto the wire [`TokenSummary`] DTO.
/// Never carries the plaintext (the backend stores only its SHA-256).
fn token_summary(record: &PatRecord) -> TokenSummary {
    TokenSummary {
        id: record.id.clone(),
        name: record.name.clone(),
        scopes: record.scopes.clone(),
        created_at: record.created_at.to_rfc3339(),
        last_used_at: record.last_used_at.map(|t| t.to_rfc3339()),
        expires_at: record.expires_at.map(|t| t.to_rfc3339()),
    }
}

/// Real org-membership count for `MeResponse.orgs_count`.
///
/// `Some(n)` from the shared [`MembershipStore`] when wired;
/// **`None` (field omitted)** when the store is absent — honest
/// degradation, never a synthesized `0` (honest capability contract / durable control queue). The
/// `usize`→`u32` cast saturates (a user with more than `u32::MAX`
/// memberships is impossible in practice, but the cast is explicit
/// rather than silently wrapping).
async fn orgs_count_for(state: &AppState, principal: &Principal) -> Result<Option<u32>, ApiError> {
    match &state.membership_store {
        Some(store) => {
            let n = store.list_orgs_for_principal(principal).await?.len();
            Ok(Some(u32::try_from(n).unwrap_or(u32::MAX)))
        },
        None => Ok(None),
    }
}

/// `GET /api/v1/me` — current user's own profile.
///
/// `tokens_count` is the real count of the caller's active PATs.
/// `orgs_count` is the **real** principal→orgs membership count from the
/// shared [`MembershipStore`](crate::state::MembershipStore) (Phase 3 —
/// see [`list_my_orgs`]); it degrades to *absent* (never a synthesized
/// `0`) only if the membership store is unwired — honest capability contract / durable control queue.
#[utoipa::path(
    get,
    path = "/me",
    tag = "me",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 200, description = "Authenticated user's profile.", body = MeResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Authenticated user no longer exists.", body = ProblemDetails),
        (status = 503, description = "Authentication backend not configured.", body = ProblemDetails),
    ),
)]
pub async fn get_me(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<MeResponse>> {
    let user_id = require_user_id(&auth)?;
    let backend = auth_backend_or_503(&state)?;

    let profile = backend.get_user_profile(&user_id).await?;
    // Saturating, honest cast: a user with > u32::MAX PATs is impossible
    // in practice, but `as u32` would silently wrap — be explicit.
    let tokens_count = u32::try_from(backend.list_pats(&user_id).await?.len()).unwrap_or(u32::MAX);
    let orgs_count = orgs_count_for(&state, &auth.principal).await?;

    tracing::info!(user_id = %user_id, orgs_count = ?orgs_count, "me profile fetched");

    Ok(Json(MeResponse {
        user_id: profile.user_id,
        email: profile.email,
        display_name: profile.display_name,
        email_verified: profile.email_verified,
        mfa_enabled: profile.mfa_enabled,
        // Real principal→orgs count from the shared MembershipStore;
        // absent (not 0) only if the store is unwired — honest capability contract.
        orgs_count,
        tokens_count,
    }))
}

/// `PATCH /api/v1/me` — partial update of the caller's own profile.
#[utoipa::path(
    patch,
    path = "/me",
    tag = "me",
    security(("bearer" = []), ("api_key" = [])),
    request_body = UpdateMeRequest,
    responses(
        (status = 200, description = "Updated profile.", body = MeResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Authenticated user no longer exists.", body = ProblemDetails),
        (status = 503, description = "Authentication backend not configured.", body = ProblemDetails),
    ),
)]
pub async fn update_me(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<UpdateMeRequest>,
) -> ApiResult<Json<MeResponse>> {
    let user_id = require_user_id(&auth)?;
    let backend = auth_backend_or_503(&state)?;

    // Request-level validation → 400 (the port's own guard is
    // defense-in-depth, but maps to 401; surface the precise 400 here).
    if let Some(name) = body.display_name.as_deref() {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return Err(ApiError::validation_message(
                "display_name must be 1..=128 non-blank characters",
            ));
        }
    }

    let patch = ProfilePatch {
        display_name: body.display_name,
        avatar_url: body.avatar_url,
    };
    let profile = backend.update_user_profile(&user_id, patch).await?;
    // Saturating, honest cast: a user with > u32::MAX PATs is impossible
    // in practice, but `as u32` would silently wrap — be explicit.
    let tokens_count = u32::try_from(backend.list_pats(&user_id).await?.len()).unwrap_or(u32::MAX);
    let orgs_count = orgs_count_for(&state, &auth.principal).await?;

    tracing::info!(user_id = %user_id, orgs_count = ?orgs_count, "me profile updated");

    Ok(Json(MeResponse {
        user_id: profile.user_id,
        email: profile.email,
        display_name: profile.display_name,
        email_verified: profile.email_verified,
        mfa_enabled: profile.mfa_enabled,
        // Real principal→orgs count from the shared MembershipStore;
        // absent (not 0) only if the store is unwired — honest capability contract.
        orgs_count,
        tokens_count,
    }))
}

/// `GET /api/v1/me/orgs` — organisations the authenticated user belongs to.
///
/// Real end-to-end (Phase 3): delegates to the shared
/// [`MembershipStore`](crate::state::MembershipStore) principal→orgs
/// enumeration — the same store [`crate::middleware::rbac`] consults, so
/// this list is exactly the set of orgs the caller can actually access.
/// Each entry carries `{ id, role }` only; `slug` is intentionally absent
/// (no `OrgId`→slug reverse directory exists — honest capability contract, see
/// [`OrgSummary`]). 503 (honest degradation) if the store is unwired.
#[utoipa::path(
    get,
    path = "/me/orgs",
    tag = "me",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 200, description = "Organisations the caller is a member of (unpaginated; bounded per user).", body = MyOrgsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 503, description = "Membership store not configured.", body = ProblemDetails),
    ),
)]
pub async fn list_my_orgs(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<MyOrgsResponse>> {
    let store = state.membership_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "membership store is not configured; org enumeration is unavailable".to_owned(),
        )
    })?;

    let orgs = store
        .list_orgs_for_principal(&auth.principal)
        .await?
        .into_iter()
        .map(|(org_id, role)| OrgSummary {
            id: org_id.to_string(),
            role: OrgRoleDto::from(role),
        })
        .collect::<Vec<_>>();

    tracing::info!(count = orgs.len(), "me orgs listed");

    Ok(Json(MyOrgsResponse { orgs }))
}

/// `GET /api/v1/me/tokens` — list the caller's personal access tokens
/// (metadata only — never the secret values).
#[utoipa::path(
    get,
    path = "/me/tokens",
    tag = "me",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 200, description = "The caller's active PAT metadata.", body = MyTokensResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Authenticated user no longer exists.", body = ProblemDetails),
        (status = 503, description = "Authentication backend not configured.", body = ProblemDetails),
    ),
)]
pub async fn list_my_tokens(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<MyTokensResponse>> {
    let user_id = require_user_id(&auth)?;
    let backend = auth_backend_or_503(&state)?;

    let tokens = backend
        .list_pats(&user_id)
        .await?
        .iter()
        .map(token_summary)
        .collect();

    tracing::info!(user_id = %user_id, "me tokens listed");

    Ok(Json(MyTokensResponse { tokens }))
}

/// `POST /api/v1/me/tokens` — create a new personal access token.
///
/// The plaintext token is returned **exactly once** in the response body
/// and is zeroized from this handler's memory immediately after the
/// response is built. The stored form is its SHA-256; subsequent
/// [`list_my_tokens`] calls expose metadata only.
#[utoipa::path(
    post,
    path = "/me/tokens",
    tag = "me",
    security(("bearer" = []), ("api_key" = [])),
    request_body = CreateTokenRequest,
    responses(
        (status = 201, description = "Token created; `token` carries the plaintext once.", body = CreateTokenResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Authenticated user no longer exists.", body = ProblemDetails),
        (status = 503, description = "Authentication backend not configured.", body = ProblemDetails),
    ),
)]
pub async fn create_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<CreateTokenRequest>,
) -> ApiResult<(StatusCode, Json<CreateTokenResponse>)> {
    let user_id = require_user_id(&auth)?;
    let backend = auth_backend_or_503(&state)?;

    // Request-level validation → 400.
    let name = body.name.trim();
    if name.is_empty() || name.len() > 128 {
        return Err(ApiError::validation_message(
            "token name must be 1..=128 non-blank characters",
        ));
    }

    let params = CreatePatParams {
        name: name.to_owned(),
        scopes: body.scopes,
        ttl_seconds: body.ttl_seconds,
    };
    let mut minted = backend.create_pat(&user_id, params).await?;

    // Build the response (the one and only plaintext exposure), then
    // zeroize the handler-held copy. The plaintext never reaches a log or
    // error path: no `tracing` call below references it, and `MintedPat`'s
    // `Debug` is not emitted anywhere.
    let response = CreateTokenResponse {
        token: minted.plaintext.clone(),
        summary: token_summary(&minted.record),
    };
    minted.plaintext.zeroize();

    tracing::info!(
        user_id = %user_id,
        pat_id = %minted.record.id,
        "me token created"
    );

    Ok((StatusCode::CREATED, Json(response)))
}

/// `DELETE /api/v1/me/tokens/{pat}` — revoke one of the caller's PATs.
///
/// Scoped to the caller: a token id that exists but belongs to a
/// different principal returns 404 (the same outcome as a missing token)
/// so PAT ownership is never disclosed across users.
#[utoipa::path(
    delete,
    path = "/me/tokens/{pat}",
    tag = "me",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("pat" = String, Path, description = "Personal access token identifier (`pat_<token>` — the `pat_`-prefixed URL-safe base64 form, not a ULID)."),
    ),
    responses(
        (status = 200, description = "Token revoked (idempotent).", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Token does not exist or is not owned by the caller.", body = ProblemDetails),
        (status = 503, description = "Authentication backend not configured.", body = ProblemDetails),
    ),
)]
pub async fn delete_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(pat_id): Path<String>,
) -> ApiResult<Json<AckResponse>> {
    let user_id = require_user_id(&auth)?;
    let backend = auth_backend_or_503(&state)?;

    backend.revoke_pat(&user_id, &pat_id).await?;

    tracing::info!(user_id = %user_id, pat_id = %pat_id, "me token revoked");

    Ok(Json(AckResponse::ok()))
}
