//! User profile endpoint handlers (global, no tenant scope).
//! Auth required but no org/workspace context needed.
//!
//! Five of the six handlers are real end-to-end against the Plane-A
//! [`AuthBackend`] port (profile read/patch + PAT list/create/revoke). The
//! sixth — [`list_my_orgs`] — is an honest 501 stub (canon §4.5): listing
//! the orgs a principal belongs to has **no** end-to-end backing today
//! (`MembershipStore` exposes only point role lookups, not principal→orgs
//! enumeration, and no `OrgRepo` impl exists). That capability lands with
//! the org/membership work (Phase 3); advertising a 200 the stack cannot
//! honor would be a false capability — worse than the honest stub.
//!
//! ## Durability (canon §11.6 / §11.5 — operator-facing)
//!
//! These endpoints are **implemented and work end-to-end**, but the only
//! wired `AuthBackend` is the in-memory one (`InMemoryAuthBackend`). All
//! `me/*` profile and PAT state is therefore **process-local: it is lost
//! on restart and is NOT shared across replicas.** A PAT minted via
//! `POST /me/tokens` stops authenticating the moment the process exits,
//! and is invisible to other instances. This is the same local-first
//! caveat the in-memory idempotency backend carries (see the
//! `crates/api/README.md` idempotency note) — it persists once a
//! storage-backed `AuthBackend` lands (no such impl exists today;
//! `nebula_storage` ships no `UserRepo`/`PatRepo`/`SessionRepo`). The
//! durability gap is strictly about persistence, not capability.
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
            TokenSummary, UpdateMeRequest,
        },
        shared::AckResponse,
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
/// returning 503 (orchestration absent — mirrors the §13-step-6 pattern
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

/// `GET /api/v1/me` — current user's own profile.
///
/// `tokens_count` is the real count of the caller's active PATs.
/// `orgs_count` is `None` — and therefore **omitted from the JSON** —
/// because principal→orgs membership enumeration is not wired end-to-end
/// until the org/membership phase (see [`list_my_orgs`]). The wire field
/// is absent rather than a synthesized `0`: a count the system cannot
/// compute would be a false value on the wire (canon §4.5 / §12.2). The
/// org/membership phase makes it `Some(n)` (additive, non-breaking).
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

    tracing::info!(user_id = %user_id, "me profile fetched");

    Ok(Json(MeResponse {
        user_id: profile.user_id,
        email: profile.email,
        display_name: profile.display_name,
        email_verified: profile.email_verified,
        mfa_enabled: profile.mfa_enabled,
        // Omitted from the wire (not a synthesized 0) until principal→orgs
        // enumeration is wired — canon §4.5 / §12.2. See the struct doc.
        orgs_count: None,
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

    tracing::info!(user_id = %user_id, "me profile updated");

    Ok(Json(MeResponse {
        user_id: profile.user_id,
        email: profile.email,
        display_name: profile.display_name,
        email_verified: profile.email_verified,
        mfa_enabled: profile.mfa_enabled,
        // Omitted from the wire (not a synthesized 0) until principal→orgs
        // enumeration is wired — canon §4.5 / §12.2. See the struct doc.
        orgs_count: None,
        tokens_count,
    }))
}

/// `GET /api/v1/me/orgs` — organisations the authenticated user belongs to.
///
/// **Honest 501 (canon §4.5).** There is no end-to-end path that
/// enumerates the orgs a principal belongs to: `MembershipStore` exposes
/// only point role lookups (`get_org_role(org_id, principal)`), not
/// principal→orgs enumeration, and `nebula_storage` ships no `OrgRepo`
/// implementation. This capability lands with the org/membership phase.
/// Returning a synthetic `{ "orgs": [] }` would advertise a list the
/// stack structurally cannot produce — a false capability, strictly worse
/// than this honest stub.
#[utoipa::path(
    get,
    path = "/me/orgs",
    tag = "me (planned)",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 501, description = "Not yet implemented; principal→orgs enumeration lands with the org/membership phase.", body = MyOrgsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
    ),
)]
#[deprecated(
    note = "Stub: principal→orgs enumeration is not wired end-to-end until the org/membership phase (canon §4.5 honest 501)."
)]
pub async fn list_my_orgs(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "principal→orgs enumeration is not wired end-to-end yet (org/membership phase)".to_string(),
    ))
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
        ("pat" = String, Path, description = "Personal access token identifier (`pat_<ULID>`)."),
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
