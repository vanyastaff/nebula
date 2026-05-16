//! Organization-level endpoint handlers (auth + org-level tenancy).
//!
//! ## §4.5 status (Phase 3, "Option 1" honest contract)
//!
//! **Graduated stub→implemented** (real end-to-end against the shared
//! [`MembershipStore`] — the same store
//! [`crate::middleware::rbac`] consults, so a write here is immediately
//! visible to the next RBAC check):
//!
//! - `GET /orgs/{org}/members` — [`list_members`]
//! - `POST /orgs/{org}/members` — [`add_member`] (direct add-by-principal,
//!   not an email invitation — see [`super::dto`] for the contract change)
//! - `DELETE /orgs/{org}/members/{principal}` — [`remove_member`]
//!
//! **Still honest 501** (canon §4.5 — shipping them would be a false
//! capability, see [`super`] module docs):
//!
//! - `GET`/`PATCH`/`DELETE /orgs/{org}` — no org-record store
//!   (name/plan/created_at). Separate `OrgStore` milestone.
//! - `GET`/`POST`/`DELETE /orgs/{org}/service-accounts` — no end-to-end
//!   `Principal::ServiceAccount` auth path; a minted SA cannot
//!   authenticate. Separate service-account-identity milestone.
//!
//! ## Authorization & abuse-safety (member mutations)
//!
//! Membership mutation is privilege-escalation territory. Every mutation:
//!
//! 1. is reachable only behind [`crate::middleware::rbac`] — a caller with
//!    no role in `{org}` gets a 404 *before* the handler (cross-tenant
//!    enumeration prevention), so the handler always sees a member caller
//!    with a concrete `org_role`;
//! 2. requires the `tenant.require(...)` org-admin gate (403 otherwise);
//! 3. **clamps the granted role to the caller's own role** — a caller can
//!    never grant (to anyone, including themselves) a role above their
//!    own;
//! 4. forbids removing/superseding a member whose role is **≥** the
//!    caller's (no lateral or upward takedown);
//! 5. forbids **any** state transition that would drop the org's
//!    privileged (`OrgOwner | OrgAdmin`) set below one — a removal **or**
//!    a privilege-reducing `add_member` upsert (self *or* cross-target;
//!    the role-precedence self-bypass does **not** bypass this) → 409.
//!    This invariant is enforced **atomically at the store seam**
//!    ([`MembershipStore::add_member_guarded`] /
//!    [`MembershipStore::remove_member_guarded`]) — count-and-mutate
//!    under one lock — so no concurrent demotion/removal can race past a
//!    handler-level check-then-act;
//! 6. treats an absent target member as a 404 identical to "no such org"
//!    so membership is never disclosed by an IDOR probe.
//!
//! ## Provisioning & durability (canon §4.5 / §11.6 / §12.5)
//!
//! These endpoints require an explicitly-provisioned `MembershipStore`.
//! The default `apps/server` binary deliberately leaves it **unwired**
//! (PR #671 P1: auto-seeding a bootstrap owner the empty default
//! `AuthBackend` could never authenticate would 404-deadlock every
//! org/workspace route — a deployment-level §4.5 false capability — and
//! a hardcoded auto-seed would be a default-credential surface). When
//! unwired, the `membership_or_503` port guard returns an honest **503**
//! (port-absent, same posture as `me/*` when `auth_backend` is absent) and
//! [`crate::middleware::rbac`] stays inert. When provisioned, the only
//! impl is the process-local in-memory
//! [`super::membership::InMemoryMembershipStore`] (memberships lost on
//! restart, not shared across replicas — same local-first caveat as
//! `me/*` and the `memory` idempotency backend). The capability is real
//! and tested end-to-end (`tests/org_e2e.rs`); see
//! `apps/server/src/compose.rs::default_state` and
//! [`super::membership`] for the provisioning contract.

use std::str::FromStr;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use nebula_core::{OrgRole, Principal, ServiceAccountId, TenantContext, UserId};

use crate::{
    domain::{
        org::dto::{
            AddMemberRequest, CreateServiceAccountRequest, CreateServiceAccountResponse,
            MemberSummary, MembersResponse, OrgResponse, ServiceAccountsResponse, UpdateOrgRequest,
        },
        shared::{AckResponse, OrgRoleDto},
    },
    error::{ApiError, ApiResult, ProblemDetails},
    state::{AppState, MembershipStore},
};

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Borrow the wired membership store, or fail closed with 503.
///
/// When the port is unwired the membership surface is genuinely absent;
/// 503 (honest degradation — same pattern as `me/*` `auth_backend_or_503`)
/// is correct, not a fabricated empty list. In practice RBAC middleware
/// would have already 404'd a request whose org is unresolved, but a
/// handler must never assume an `Option` is `Some`.
fn membership_or_503(state: &AppState) -> Result<&std::sync::Arc<dyn MembershipStore>, ApiError> {
    state.membership_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "membership store is not configured; org member endpoints are unavailable".to_owned(),
        )
    })
}

/// Parse a wire principal-id (`usr_<ULID>` / `svc_<ULID>`) into a
/// [`Principal`]. Returns a 400 for anything else — we never coerce an
/// unparsable identity into a guessed principal (canon §4.5).
fn parse_member_principal(raw: &str) -> Result<Principal, ApiError> {
    if let Ok(uid) = UserId::from_str(raw) {
        return Ok(Principal::User(uid));
    }
    if let Ok(sid) = ServiceAccountId::from_str(raw) {
        return Ok(Principal::ServiceAccount(sid));
    }
    Err(ApiError::validation_message(format!(
        "principal_id must be a `usr_<ULID>` or `svc_<ULID>` identity; got {raw:?}"
    )))
}

/// The user-facing identity string for a member-row [`Principal`] — the
/// exact value `DELETE /orgs/{org}/members/{principal}` accepts back.
///
/// Member rows are **only ever** `User`/`ServiceAccount`: every request
/// path constructs the principal via [`parse_member_principal`], which
/// rejects everything else, and the only other writer (`seed`) is a
/// test/bootstrap path. `Workflow`/`System` would not round-trip through
/// `parse_member_principal` (a bare `wf_…` id or the literal `"system"`
/// is not a `usr_`/`svc_` identity), so emitting one would put an
/// unparsable `principal_id` on the wire. That is unreachable today;
/// `debug_assert!` makes a future seeding path that violates the
/// invariant fail loudly in tests rather than silently shipping a
/// dangling id, and the release fallback is an explicit, obviously-
/// invalid sentinel (never a half-valid bare id).
fn principal_id_string(p: &Principal) -> String {
    match p {
        Principal::User(id) => id.to_string(),
        Principal::ServiceAccount(id) => id.to_string(),
        other => {
            debug_assert!(
                false,
                "org member row holds a non-User/ServiceAccount principal \
                 ({other:?}) — member rows must be usr_/svc_ only; a seeding \
                 path violated the invariant"
            );
            "invalid:non-member-principal".to_owned()
        },
    }
}

/// The caller's effective org role, as resolved by RBAC middleware.
///
/// `Internal` (not 403) when absent: with a wired store RBAC guarantees a
/// concrete `org_role` reaches the handler (it 404s otherwise), so a
/// missing role here is an invariant breach, not a client error.
fn caller_role(tenant: &TenantContext) -> Result<OrgRole, ApiError> {
    tenant.org_role.ok_or_else(|| {
        ApiError::Internal(
            "org member handler reached without a resolved caller org role".to_owned(),
        )
    })
}

// ── Org-record endpoints — honest 501 (no org-record store) ──────────────────

/// `GET /api/v1/orgs/{org}` — organisation details.
#[utoipa::path(
    get,
    path = "/orgs/{org}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under the org-record store milestone.", body = OrgResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Organisation not found.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 until the org-record store milestone closes.")]
pub async fn get_org(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "org-record store not implemented — tracked under ADR-0047 Stub Endpoint Policy"
            .to_string(),
    ))
}

/// `PATCH /api/v1/orgs/{org}` — update organisation settings (requires `OrgUpdate`).
#[utoipa::path(
    patch,
    path = "/orgs/{org}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    request_body = UpdateOrgRequest,
    responses(
        (status = 501, description = "Not yet implemented; tracked under the org-record store milestone.", body = OrgResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `OrgUpdate` permission (gate is enforced today).", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 until the org-record store milestone closes.")]
pub async fn update_org(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::OrgUpdate)?;
    Err(ApiError::NotImplemented(
        "org-record store not implemented — tracked under ADR-0047 Stub Endpoint Policy"
            .to_string(),
    ))
}

/// `DELETE /api/v1/orgs/{org}` — delete organisation (requires `OrgDelete`).
#[utoipa::path(
    delete,
    path = "/orgs/{org}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under the org-record store milestone.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `OrgDelete` permission (gate is enforced today).", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 until the org-record store milestone closes.")]
pub async fn delete_org(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::OrgDelete)?;
    Err(ApiError::NotImplemented(
        "org-record store not implemented — tracked under ADR-0047 Stub Endpoint Policy"
            .to_string(),
    ))
}

// ── Member endpoints — graduated stub→implemented ────────────────────────────

/// `GET /api/v1/orgs/{org}/members` — list organisation members.
///
/// Reachable only behind RBAC (a non-member caller is 404'd before this
/// handler). Any org member may list — the read is gated by RBAC
/// membership itself, matching the `members:read` semantics. The response
/// carries only role-index fields (no `email`/`joined_at` — canon §4.5).
#[utoipa::path(
    get,
    path = "/orgs/{org}/members",
    tag = "orgs",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 200, description = "Org members (role index; bounded, unpaginated).", body = MembersResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller is not a member of this organisation.", body = ProblemDetails),
        (status = 404, description = "Organisation not found / caller has no access.", body = ProblemDetails),
        (status = 503, description = "Membership store not configured.", body = ProblemDetails),
    ),
)]
pub async fn list_members(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<MembersResponse>> {
    let store = membership_or_503(&state)?;
    // Caller is guaranteed an org member by RBAC; assert the invariant.
    let _ = caller_role(&tenant)?;

    let members = store
        .list_members(tenant.org_id)
        .await?
        .into_iter()
        .map(|m| MemberSummary {
            principal_id: principal_id_string(&m.principal),
            role: OrgRoleDto::from(m.role),
        })
        .collect::<Vec<_>>();

    tracing::info!(
        org_id = %tenant.org_id,
        count = members.len(),
        "org members listed"
    );

    Ok(Json(MembersResponse { members }))
}

/// `POST /api/v1/orgs/{org}/members` — **direct add-by-principal-id**
/// (requires `MemberInvite` → `OrgAdmin`).
///
/// Abuse-safety (see module docs): admin gate, granted role clamped to the
/// caller's own role, a caller cannot grant a role **above** their own to
/// anyone (including themselves), and cannot supersede a ≥-privileged
/// member. Idempotent: re-adding an existing principal updates their role.
/// The **org-lockout invariant** (a privilege-reducing upsert — self OR
/// cross-target — that would zero the `OrgOwner`/`OrgAdmin` set is
/// refused with 409) is enforced atomically at the store seam
/// ([`MembershipStore::add_member_guarded`]), not by a handler-level
/// check-then-act, so no concurrent demotion can race past it.
#[utoipa::path(
    post,
    path = "/orgs/{org}/members",
    tag = "orgs",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    request_body = AddMemberRequest,
    responses(
        (status = 201, description = "Member added (or role updated).", body = MemberSummary),
        (status = 400, description = "Unparsable principal id or unknown role token.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `MemberInvite` (OrgAdmin) or attempted privilege escalation.", body = ProblemDetails),
        (status = 404, description = "Organisation not found / caller has no access.", body = ProblemDetails),
        (status = 409, description = "Refused: the (demoting) upsert would remove the last org owner/admin (org lockout).", body = ProblemDetails),
        (status = 503, description = "Membership store not configured.", body = ProblemDetails),
    ),
)]
pub async fn add_member(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(body): Json<AddMemberRequest>,
) -> ApiResult<(StatusCode, Json<MemberSummary>)> {
    let store = membership_or_503(&state)?;
    tenant.require(nebula_core::Permission::MemberInvite)?;
    let caller = caller_role(&tenant)?;

    // Validate the request shape → 400 (not 403): a malformed body is a
    // client error, distinct from a privilege violation.
    let target_principal = parse_member_principal(&body.principal_id)?;
    let granted = OrgRoleDto::parse(&body.role.0).ok_or_else(|| {
        ApiError::validation_message(format!(
            "role must be one of member|billing|admin|owner; got {:?}",
            body.role.0
        ))
    })?;

    // Abuse guard 1 — role clamp: cannot grant a role above the caller's
    // own (prevents an admin minting an owner, or any self-escalation).
    if granted > caller {
        return Err(ApiError::Forbidden(format!(
            "cannot grant role {} above your own role {}",
            OrgRoleDto::token(granted),
            OrgRoleDto::token(caller)
        )));
    }

    // Abuse guard 2 — cannot supersede a member whose current role is ≥
    // the caller's (no lateral/upward takedown via re-add/downgrade).
    // This is a *policy* check (needs caller context); racing it is
    // benign — the lockout *invariant* is enforced atomically at the
    // store seam below regardless of this read's freshness.
    if let Some(existing) = store.get_org_role(tenant.org_id, &target_principal).await?
        && existing >= caller
        && target_principal != tenant.principal
    {
        return Err(ApiError::Forbidden(format!(
            "cannot modify a member whose role {} is at or above your own role {}",
            OrgRoleDto::token(existing),
            OrgRoleDto::token(caller)
        )));
    }

    // Org-lockout invariant + the write happen atomically under one store
    // lock. This covers the privilege-reducing upsert of the LAST
    // privileged principal — whether that is the caller demoting
    // themselves (the self-bypass on guard 2 does NOT bypass this) or a
    // cross-target demotion — and is race-free (no handler check-then-act
    // window).
    match store
        .add_member_guarded(tenant.org_id, &target_principal, granted)
        .await?
    {
        crate::state::AddMemberOutcome::Added => {},
        crate::state::AddMemberOutcome::WouldLockOut => {
            return Err(ApiError::Conflict(
                "refused: this role change would remove the last org owner/admin and \
                 lock the organisation out"
                    .to_owned(),
            ));
        },
    }

    tracing::info!(
        org_id = %tenant.org_id,
        principal = %body.principal_id,
        role = OrgRoleDto::token(granted),
        "org member added/updated"
    );

    Ok((
        StatusCode::CREATED,
        Json(MemberSummary {
            principal_id: principal_id_string(&target_principal),
            role: OrgRoleDto::from(granted),
        }),
    ))
}

/// `DELETE /api/v1/orgs/{org}/members/{principal}` — remove a member
/// (requires `MemberRemove` → `OrgAdmin`).
///
/// Abuse-safety (see module docs): admin gate; cannot remove a member
/// whose role is **≥** the caller's (except self-removal of a non-last
/// admin); cannot remove the **last** `OrgOwner`/`OrgAdmin` (org-lockout
/// → 409); a non-member target is a 404 identical to "no such org" so
/// membership is not disclosed by IDOR.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/members/{principal}",
    tag = "orgs",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("principal" = String, Path, description = "Principal identity (`usr_<ULID>` / `svc_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Member removed.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `MemberRemove` (OrgAdmin) or attempted to remove a ≥-privileged member.", body = ProblemDetails),
        (status = 404, description = "Member does not exist in this organisation (or caller has no access).", body = ProblemDetails),
        (status = 409, description = "Refused: would remove the last org owner/admin (org lockout).", body = ProblemDetails),
        (status = 503, description = "Membership store not configured.", body = ProblemDetails),
    ),
)]
pub async fn remove_member(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, principal_id)): Path<(String, String)>,
) -> ApiResult<Json<AckResponse>> {
    let store = membership_or_503(&state)?;
    tenant.require(nebula_core::Permission::MemberRemove)?;
    let caller = caller_role(&tenant)?;

    let target_principal = parse_member_principal(&principal_id)?;

    // IDOR-safe: a non-member target is indistinguishable from "no such
    // org" — 404, never a disclosure of who is/ isn't a member.
    let Some(target_role) = store.get_org_role(tenant.org_id, &target_principal).await? else {
        return Err(ApiError::NotFound("member not found".to_owned()));
    };

    let is_self = target_principal == tenant.principal;

    // Abuse guard 1 — cannot remove a member at or above your own role,
    // *unless* it is yourself (an admin may always leave, subject to the
    // last-admin guard below).
    if target_role >= caller && !is_self {
        return Err(ApiError::Forbidden(format!(
            "cannot remove a member whose role {} is at or above your own role {}",
            OrgRoleDto::token(target_role),
            OrgRoleDto::token(caller)
        )));
    }

    // Abuse guard 2 — org-lockout — AND the removal itself, atomically at
    // the store seam. The privileged count and the delete happen under
    // one lock, so two concurrent removals of the last two admins cannot
    // both observe `privileged == 2` and both delete (the TOCTOU the
    // old per-handler count had). A membership re-check inside the lock
    // collapses an existence race to the same IDOR-safe 404.
    match store
        .remove_member_guarded(tenant.org_id, &target_principal)
        .await?
    {
        crate::state::RemoveMemberOutcome::Removed => {},
        crate::state::RemoveMemberOutcome::NotFound => {
            return Err(ApiError::NotFound("member not found".to_owned()));
        },
        crate::state::RemoveMemberOutcome::WouldLockOut => {
            return Err(ApiError::Conflict(
                "refused: removing the last org owner/admin would lock the organisation out"
                    .to_owned(),
            ));
        },
    }

    tracing::info!(
        org_id = %tenant.org_id,
        principal = %principal_id,
        removed_role = OrgRoleDto::token(target_role),
        self_removal = is_self,
        "org member removed"
    );

    Ok(Json(AckResponse::ok()))
}

// ── Service-account endpoints — honest 501 (no SA auth path) ─────────────────

/// `GET /api/v1/orgs/{org}/service-accounts` — list service accounts.
#[utoipa::path(
    get,
    path = "/orgs/{org}/service-accounts",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under the service-account identity milestone.", body = ServiceAccountsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller is not a member of this organisation.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 until end-to-end service-account identity exists.")]
pub async fn list_service_accounts(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "service-account identity not implemented — tracked under ADR-0047 Stub Endpoint Policy"
            .to_string(),
    ))
}

/// `POST /api/v1/orgs/{org}/service-accounts` — create a service account
/// (requires `ServiceAccountManage`).
#[utoipa::path(
    post,
    path = "/orgs/{org}/service-accounts",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    request_body = CreateServiceAccountRequest,
    responses(
        (status = 501, description = "Not yet implemented; tracked under the service-account identity milestone.", body = CreateServiceAccountResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `ServiceAccountManage` permission (gate is enforced today).", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 until end-to-end service-account identity exists.")]
pub async fn create_service_account(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::ServiceAccountManage)?;
    Err(ApiError::NotImplemented(
        "service-account identity not implemented — tracked under ADR-0047 Stub Endpoint Policy"
            .to_string(),
    ))
}

/// `DELETE /api/v1/orgs/{org}/service-accounts/{sa}` — delete a service
/// account (requires `ServiceAccountManage`).
#[utoipa::path(
    delete,
    path = "/orgs/{org}/service-accounts/{sa}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("sa" = String, Path, description = "Service-account identifier (`sa_<ULID>`)."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under the service-account identity milestone.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `ServiceAccountManage` permission (gate is enforced today).", body = ProblemDetails),
        (status = 404, description = "Service account does not exist.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 until end-to-end service-account identity exists.")]
pub async fn delete_service_account(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _sa_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::ServiceAccountManage)?;
    Err(ApiError::NotImplemented(
        "service-account identity not implemented — tracked under ADR-0047 Stub Endpoint Policy"
            .to_string(),
    ))
}
