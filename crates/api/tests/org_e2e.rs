//! `org/*` member-management end-to-end coverage (Phase 3).
//!
//! Three member endpoints graduated stub→implemented against the **shared**
//! `InMemoryMembershipStore` (the same `Arc` `rbac_middleware` consults):
//! `GET`/`POST`/`DELETE` under `…/orgs/{org}/members`. These tests drive
//! the full middleware → RBAC → handler → store path against a real
//! in-memory backing (not a mock), reusing the `common::org_support`
//! harness (no fixture duplication — Phase-1/2 shared-harness rule).
//!
//! The org-record (`GET`/`PATCH`/`DELETE /orgs/{org}`) and service-account
//! endpoints stay honest-501; their 501 contract is locked by
//! `openapi_canon_compliance.rs` and not re-tested here.
//!
//! ## Coverage
//!
//! | Concern | Test |
//! |---------|------|
//! | RBAC-coherence (one shared store) | `added_member_is_immediately_rbac_authorized` |
//! | list happy path | `list_members_returns_seeded_admin` |
//! | add happy path (201) | `add_member_grants_role_and_is_listed` |
//! | add idempotent upsert | `add_member_upsert_updates_role` |
//! | add role-clamp (no escalation) | `admin_cannot_grant_owner_role` |
//! | add cannot supersede ≥-peer | `admin_cannot_modify_equal_or_higher_member` |
//! | add bad principal → 400 | `add_member_bad_principal_is_400` |
//! | add bad role token → 400 | `add_member_unknown_role_is_400` |
//! | **add self-demote lockout → 409** | `sole_owner_cannot_self_demote_via_add` |
//! | **add cross-demote lockout → 409** | `cannot_demote_second_to_last_privileged_via_add` |
//! | **add demote non-last privileged OK** | `owner_can_demote_when_another_admin_remains` |
//! | non-admin denied (403) | `non_admin_member_cannot_add` |
//! | non-member → RBAC 404 | `non_member_caller_is_404_not_403` |
//! | remove happy path | `remove_member_deletes_and_revokes_rbac` |
//! | remove last admin → 409 | `cannot_remove_last_owner_admin` |
//! | remove ≥-privileged → 403 | `admin_cannot_remove_owner` |
//! | remove IDOR → 404 no-disclosure | `remove_unknown_member_is_404` |
//! | unauth → 401 | `member_endpoints_require_auth` |
//!
//! Atomic store-seam concurrency (TOCTOU) coverage lives as store-level
//! unit tests in `domain::org::membership::tests` (true concurrent
//! drives of the guarded methods against the shared store):
//! `concurrent_remove_of_last_two_admins_keeps_one`,
//! `concurrent_demote_and_remove_keeps_one_admin`.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    TEST_CSRF_COOKIE, TEST_CSRF_TOKEN, TEST_ORG,
    org_support::{OrgActor, create_org_state, create_org_state_with_role, seed_member},
};
use nebula_api::{ApiConfig, app, state::MembershipStore};
use nebula_core::{OrgRole, Principal, UserId};
use serde_json::Value;
use tower::ServiceExt;

fn members_path() -> String {
    format!("/api/v1/orgs/{TEST_ORG}/members")
}

fn member_path(principal_id: &str) -> String {
    format!("/api/v1/orgs/{TEST_ORG}/members/{principal_id}")
}

fn get(uri: &str, jwt: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::empty())
        .unwrap()
}

/// State-changing request with the double-submit CSRF pair the JWT auth
/// path requires (identical contract to the Phase-1/2 mutating helper).
fn mutating(method: &str, uri: &str, jwt: &str, json_body: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {jwt}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE);
    let body = match json_body {
        Some(j) => {
            b = b.header("content-type", "application/json");
            Body::from(j.to_owned())
        },
        None => Body::empty(),
    };
    b.body(body).unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body readable");
    serde_json::from_slice(&bytes).expect("body is JSON")
}

fn ct_is_problem(response: &axum::response::Response) -> bool {
    response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/problem+json"))
        .unwrap_or(false)
}

// ── RBAC-coherence: one shared store ─────────────────────────────────────────

/// THE load-bearing test: a principal added via `POST /members` is
/// authorized by `rbac_middleware` on the *very next* request — proving
/// the org handler and RBAC read the same `Arc<dyn MembershipStore>`
/// (no propagation window, no second store).
#[tokio::test]
async fn added_member_is_immediately_rbac_authorized() {
    let (state, _store, admin) = create_org_state();
    let api_config = ApiConfig::for_test();

    // A brand-new principal: not a member → RBAC must 404 it.
    let newcomer = OrgActor::new_user();
    let app = app::build_app(state.clone(), &api_config);
    let pre = app
        .oneshot(get(&members_path(), &newcomer.jwt))
        .await
        .unwrap();
    assert_eq!(
        pre.status(),
        StatusCode::NOT_FOUND,
        "a non-member must be RBAC-404'd before the handler (enumeration prevention)"
    );

    // Admin adds the newcomer as a plain member.
    let app = app::build_app(state.clone(), &api_config);
    let add = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &admin.jwt,
            Some(&format!(
                r#"{{"principal_id":"{}","role":"member"}}"#,
                newcomer.user_id
            )),
        ))
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::CREATED);

    // The SAME newcomer JWT now passes RBAC and lists members — proving
    // the write landed in the store RBAC consults.
    let app = app::build_app(state, &api_config);
    let post = app
        .oneshot(get(&members_path(), &newcomer.jwt))
        .await
        .unwrap();
    assert_eq!(
        post.status(),
        StatusCode::OK,
        "the just-added member must be immediately authorized by RBAC \
         (one shared store — RBAC coherence)"
    );
}

// ── list_members ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_members_returns_seeded_admin() {
    let (state, _store, admin) = create_org_state();
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app.oneshot(get(&members_path(), &admin.jwt)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let members = body["members"].as_array().expect("members array");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["principal_id"], admin.user_id);
    assert_eq!(members[0]["role"], "admin");
    // §4.5: dropped fields must NOT reappear.
    assert!(
        members[0].get("email").is_none() && members[0].get("joined_at").is_none(),
        "MemberSummary must not carry synthesized email/joined_at"
    );
}

// ── add_member ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn add_member_grants_role_and_is_listed() {
    let (state, _store, admin) = create_org_state();
    let api_config = ApiConfig::for_test();
    let newcomer = UserId::new().to_string();

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &admin.jwt,
            Some(&format!(
                r#"{{"principal_id":"{newcomer}","role":"member"}}"#
            )),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    assert_eq!(body["principal_id"], newcomer);
    assert_eq!(body["role"], "member");

    // Now visible in the list.
    let app = app::build_app(state, &api_config);
    let listed = body_json(app.oneshot(get(&members_path(), &admin.jwt)).await.unwrap()).await;
    let ids: Vec<&str> = listed["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["principal_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&newcomer.as_str()));
}

#[tokio::test]
async fn add_member_upsert_updates_role() {
    let (state, store, admin) = create_org_state_with_role(OrgRole::OrgOwner);
    let api_config = ApiConfig::for_test();
    let target = UserId::new();
    seed_member(&store, Principal::User(target), OrgRole::OrgMember).await;

    // Owner promotes the member to admin via re-add (idempotent upsert).
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &admin.jwt,
            Some(&format!(r#"{{"principal_id":"{target}","role":"admin"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        store
            .get_org_role(TEST_ORG.parse().unwrap(), &Principal::User(target))
            .await
            .unwrap(),
        Some(OrgRole::OrgAdmin),
        "re-add must upsert the role in the shared store"
    );
}

#[tokio::test]
async fn admin_cannot_grant_owner_role() {
    // Abuse: an OrgAdmin must not be able to mint an OrgOwner (role-clamp
    // — prevents privilege escalation, incl. self-escalation).
    let (state, _store, admin) = create_org_state(); // admin == OrgAdmin
    let api_config = ApiConfig::for_test();
    let victim = UserId::new().to_string();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &admin.jwt,
            Some(&format!(r#"{{"principal_id":"{victim}","role":"owner"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an OrgAdmin granting OrgOwner is privilege escalation → 403"
    );
    assert!(ct_is_problem(&response));
}

#[tokio::test]
async fn admin_cannot_modify_equal_or_higher_member() {
    // Abuse: cannot supersede (downgrade/re-add) a member whose current
    // role is ≥ the caller's. Seed a *second* admin; the first admin must
    // not be able to demote them.
    let (state, store, admin) = create_org_state(); // OrgAdmin
    let api_config = ApiConfig::for_test();
    let peer = UserId::new();
    seed_member(&store, Principal::User(peer), OrgRole::OrgAdmin).await;

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &admin.jwt,
            Some(&format!(r#"{{"principal_id":"{peer}","role":"member"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an admin cannot demote an equal-privileged admin"
    );
}

#[tokio::test]
async fn sole_owner_cannot_self_demote_via_add() {
    // C1 regression: the sole OrgOwner POSTs {self, "member"}. role-clamp
    // (member <= owner) passes and the role-precedence self-bypass passes
    // — the ONLY thing standing between this and a permanent org lockout
    // is the add-path org-lockout invariant. Must be 409, and the org
    // must still be administerable afterwards.
    let (state, _store, owner) = create_org_state_with_role(OrgRole::OrgOwner);
    let api_config = ApiConfig::for_test();

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &owner.jwt,
            Some(&format!(
                r#"{{"principal_id":"{}","role":"member"}}"#,
                owner.user_id
            )),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "the sole owner self-demoting to member would zero the privileged \
         set — must be refused 409 (org-lockout), NOT silently applied"
    );
    assert!(ct_is_problem(&response));

    // Prove the org is NOT locked out: the owner can still perform an
    // admin-gated mutation (add another member) afterwards.
    let app = app::build_app(state, &api_config);
    let still_admin = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &owner.jwt,
            Some(&format!(
                r#"{{"principal_id":"{}","role":"member"}}"#,
                UserId::new()
            )),
        ))
        .await
        .unwrap();
    assert_eq!(
        still_admin.status(),
        StatusCode::CREATED,
        "owner must retain admin power — the self-demote was correctly rejected"
    );
}

#[tokio::test]
async fn cannot_demote_second_to_last_privileged_via_add() {
    // I1 regression (cross-target): an OrgOwner demoting the only OTHER
    // privileged principal when they themselves are NOT privileged would
    // zero the privileged set. Seed: caller is OrgOwner (privileged) +
    // one OrgAdmin. Owner demoting the admin is fine (owner still
    // privileged). The real cross-target lockout is: a privileged caller
    // who is about to also lose privilege — modelled here by a non-owner
    // path is impossible (caller must be admin+ to reach the handler), so
    // the precise cross-target case is "the last privileged besides the
    // caller, and the write also drops the caller". Simpler faithful
    // model: two admins, demote one — must SUCCEED (still one left); then
    // demoting the last remaining privileged must 409.
    let (state, store, owner) = create_org_state_with_role(OrgRole::OrgOwner);
    let api_config = ApiConfig::for_test();
    let admin2 = UserId::new();
    seed_member(&store, Principal::User(admin2), OrgRole::OrgAdmin).await;

    // Demote admin2 → member: still leaves the owner privileged → OK.
    let app = app::build_app(state.clone(), &api_config);
    let ok = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &owner.jwt,
            Some(&format!(r#"{{"principal_id":"{admin2}","role":"member"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(
        ok.status(),
        StatusCode::CREATED,
        "demoting a non-last privileged member is allowed (owner remains)"
    );

    // Now the owner is the LAST privileged principal. Owner self-demote
    // → 409 (this is the cross-/self- last-privileged invariant; the
    // store seam decides on the post-write privileged count, not on who
    // the target is).
    let app = app::build_app(state, &api_config);
    let locked = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &owner.jwt,
            Some(&format!(
                r#"{{"principal_id":"{}","role":"member"}}"#,
                owner.user_id
            )),
        ))
        .await
        .unwrap();
    assert_eq!(
        locked.status(),
        StatusCode::CONFLICT,
        "once the owner is the last privileged principal, demoting them \
         (any target) must be refused 409"
    );
}

#[tokio::test]
async fn owner_can_demote_when_another_admin_remains() {
    // Positive control: the lockout guard must NOT over-block. With two
    // privileged principals, demoting one to member succeeds and the
    // store reflects it (the guard keys on the post-write count, not on
    // "is the target privileged").
    let (state, store, owner) = create_org_state_with_role(OrgRole::OrgOwner);
    let api_config = ApiConfig::for_test();
    let admin2 = UserId::new();
    seed_member(&store, Principal::User(admin2), OrgRole::OrgAdmin).await;

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &owner.jwt,
            Some(&format!(r#"{{"principal_id":"{admin2}","role":"member"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        store
            .get_org_role(TEST_ORG.parse().unwrap(), &Principal::User(admin2))
            .await
            .unwrap(),
        Some(OrgRole::OrgMember),
        "demotion must actually apply when another admin remains"
    );
}

#[tokio::test]
async fn add_member_bad_principal_is_400() {
    let (state, _store, admin) = create_org_state();
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &admin.jwt,
            Some(r#"{"principal_id":"not-a-ulid","role":"member"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(ct_is_problem(&response));
}

#[tokio::test]
async fn add_member_unknown_role_is_400() {
    let (state, _store, admin) = create_org_state();
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let who = UserId::new().to_string();

    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &admin.jwt,
            Some(&format!(r#"{{"principal_id":"{who}","role":"superuser"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an unknown role token must be a 400, never a silent coercion"
    );
}

#[tokio::test]
async fn non_admin_member_cannot_add() {
    // A plain OrgMember is a member (RBAC lets them through) but the
    // `MemberInvite` gate (OrgAdmin) must still 403 the mutation.
    let (state, store, _admin) = create_org_state();
    let api_config = ApiConfig::for_test();
    let plain = OrgActor::new_user();
    seed_member(&store, plain.principal.clone(), OrgRole::OrgMember).await;

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            &members_path(),
            &plain.jwt,
            Some(&format!(
                r#"{{"principal_id":"{}","role":"member"}}"#,
                UserId::new()
            )),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a non-admin member must be 403'd by the MemberInvite gate"
    );
}

#[tokio::test]
async fn non_member_caller_is_404_not_403() {
    // Cross-tenant isolation: a caller with no role in the org is 404'd
    // by RBAC *before* the handler — membership is never disclosed.
    let (state, _store, _admin) = create_org_state();
    let api_config = ApiConfig::for_test();
    let stranger = OrgActor::new_user();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(get(&members_path(), &stranger.jwt))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a non-member must get RBAC 404 (not 403 — no membership disclosure)"
    );
}

// ── remove_member ────────────────────────────────────────────────────────────

#[tokio::test]
async fn remove_member_deletes_and_revokes_rbac() {
    let (state, store, admin) = create_org_state_with_role(OrgRole::OrgOwner);
    let api_config = ApiConfig::for_test();
    let target = OrgActor::new_user();
    seed_member(&store, target.principal.clone(), OrgRole::OrgMember).await;

    // Remove.
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(mutating(
            "DELETE",
            &member_path(&target.user_id),
            &admin.jwt,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["ok"], true);

    // The removed principal is now RBAC-404'd (revocation is immediate in
    // the shared store).
    let app = app::build_app(state, &api_config);
    let after = app
        .oneshot(get(&members_path(), &target.jwt))
        .await
        .unwrap();
    assert_eq!(
        after.status(),
        StatusCode::NOT_FOUND,
        "a removed member must immediately lose RBAC access"
    );
}

#[tokio::test]
async fn cannot_remove_last_owner_admin() {
    // Org-lockout guard: the seeded admin is the ONLY privileged
    // principal; removing themselves must be refused with 409.
    let (state, _store, admin) = create_org_state(); // sole OrgAdmin
    let api_config = ApiConfig::for_test();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "DELETE",
            &member_path(&admin.user_id),
            &admin.jwt,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "removing the last org owner/admin must be refused (org-lockout → 409)"
    );
    assert!(ct_is_problem(&response));
}

#[tokio::test]
async fn admin_cannot_remove_owner() {
    // Abuse: an OrgAdmin must not be able to remove an OrgOwner (role
    // precedence — cannot take down a ≥-privileged member).
    let (state, store, admin) = create_org_state(); // OrgAdmin
    let api_config = ApiConfig::for_test();
    let owner = OrgActor::new_user();
    seed_member(&store, owner.principal.clone(), OrgRole::OrgOwner).await;

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "DELETE",
            &member_path(&owner.user_id),
            &admin.jwt,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an OrgAdmin removing an OrgOwner is a privilege violation → 403"
    );
}

#[tokio::test]
async fn remove_unknown_member_is_404() {
    // IDOR-safe: a well-formed but non-member principal is a 404
    // identical to "no such org" — membership is not disclosed.
    let (state, _store, admin) = create_org_state();
    let api_config = ApiConfig::for_test();
    let ghost = UserId::new().to_string();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating("DELETE", &member_path(&ghost), &admin.jwt, None))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "removing a non-member must be a clean 404 (no membership disclosure)"
    );
}

// ── auth ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn member_endpoints_require_auth() {
    let (state, _store, _admin) = create_org_state();
    let api_config = ApiConfig::for_test();

    for (method, body) in [("GET", None), ("POST", Some("{}"))] {
        let app = app::build_app(state.clone(), &api_config);
        let mut req = Request::builder().method(method).uri(members_path());
        if let Some(b) = body {
            req = req.header("content-type", "application/json");
            let response = app.oneshot(req.body(Body::from(b)).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        } else {
            let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }
}
