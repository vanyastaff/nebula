//! End-to-end coverage for parent-qualified workspace membership routes.

mod common;

use axum::http::StatusCode;
use common::{
    TEST_ORG, TEST_WS,
    http_helpers::{auth_get, body_json, mutating},
    org_support::{OrgActor, create_org_state, seed_member},
};
use nebula_api::{ApiConfig, app};
use nebula_core::OrgRole;
use tower::ServiceExt;

fn collection_path() -> String {
    format!("/api/v1/orgs/{TEST_ORG}/workspaces/{TEST_WS}/members")
}

fn member_path(principal: &str) -> String {
    format!("{}/{principal}", collection_path())
}

#[tokio::test]
async fn put_is_idempotent_replacement_and_get_is_deterministic() {
    let (state, store, admin) = create_org_state();
    let first = OrgActor::new_user();
    let second = OrgActor::new_user();
    seed_member(&store, first.principal.clone(), OrgRole::OrgMember).await;
    seed_member(&store, second.principal.clone(), OrgRole::OrgMember).await;
    let config = ApiConfig::for_test();

    for (principal, role) in [(&second.user_id, "viewer"), (&first.user_id, "runner")] {
        let response = app::build_app(state.clone(), &config)
            .oneshot(mutating(
                "PUT",
                &member_path(principal),
                &admin.jwt,
                Some(&format!(r#"{{"role":"{role}"}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    // The same item URI replaces the grant instead of creating a duplicate.
    let replaced = app::build_app(state.clone(), &config)
        .oneshot(mutating(
            "PUT",
            &member_path(&first.user_id),
            &admin.jwt,
            Some(r#"{"role":"editor"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(replaced.status(), StatusCode::OK);
    assert_eq!(body_json(replaced).await["role"], "editor");

    let listed = app::build_app(state, &config)
        .oneshot(auth_get(&collection_path(), &admin.jwt))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body = body_json(listed).await;
    let members = body["members"].as_array().expect("members array");
    assert_eq!(members.len(), 2);
    assert!(
        members.windows(2).all(|pair| {
            pair[0]["principal_id"].as_str().expect("principal id")
                <= pair[1]["principal_id"].as_str().expect("principal id")
        }),
        "list order must be deterministic by principal id"
    );
    let first_row = members
        .iter()
        .find(|row| row["principal_id"] == first.user_id)
        .expect("first member listed once");
    assert_eq!(first_row["role"], "editor");
}

#[tokio::test]
async fn grant_is_immediately_used_by_rbac_and_delete_revokes_it() {
    let (state, store, admin) = create_org_state();
    let target = OrgActor::new_user();
    seed_member(&store, target.principal.clone(), OrgRole::OrgMember).await;
    let config = ApiConfig::for_test();

    let before = app::build_app(state.clone(), &config)
        .oneshot(auth_get(&collection_path(), &target.jwt))
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::NOT_FOUND);

    let put = app::build_app(state.clone(), &config)
        .oneshot(mutating(
            "PUT",
            &member_path(&target.user_id),
            &admin.jwt,
            Some(r#"{"role":"viewer"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    let authorized = app::build_app(state.clone(), &config)
        .oneshot(auth_get(&collection_path(), &target.jwt))
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);

    let deleted = app::build_app(state.clone(), &config)
        .oneshot(mutating(
            "DELETE",
            &member_path(&target.user_id),
            &admin.jwt,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);

    let after = app::build_app(state, &config)
        .oneshot(auth_get(&collection_path(), &target.jwt))
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mutation_requires_admin_and_existing_org_member() {
    let (state, store, admin) = create_org_state();
    let workspace_admin = OrgActor::new_user();
    let ordinary = OrgActor::new_user();
    seed_member(
        &store,
        workspace_admin.principal.clone(),
        OrgRole::OrgMember,
    )
    .await;
    seed_member(&store, ordinary.principal.clone(), OrgRole::OrgMember).await;
    nebula_api::state::MembershipStore::upsert_workspace_member(
        store.as_ref(),
        TEST_ORG.parse().unwrap(),
        TEST_WS.parse().unwrap(),
        &workspace_admin.principal,
        nebula_core::WorkspaceRole::WorkspaceAdmin,
    )
    .await
    .unwrap();
    let config = ApiConfig::for_test();

    let missing = OrgActor::new_user();
    let response = app::build_app(state.clone(), &config)
        .oneshot(mutating(
            "PUT",
            &member_path(&missing.user_id),
            &workspace_admin.jwt,
            Some(r#"{"role":"viewer"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let denied = app::build_app(state, &config)
        .oneshot(mutating(
            "PUT",
            &member_path(&admin.user_id),
            &ordinary.jwt,
            Some(r#"{"role":"viewer"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_identity_and_role_are_rejected() {
    let (state, _store, admin) = create_org_state();
    let config = ApiConfig::for_test();

    let bad_id = app::build_app(state.clone(), &config)
        .oneshot(mutating(
            "PUT",
            &member_path("not-an-id"),
            &admin.jwt,
            Some(r#"{"role":"viewer"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(bad_id.status(), StatusCode::BAD_REQUEST);

    let target = OrgActor::new_user();
    let bad_role = app::build_app(state, &config)
        .oneshot(mutating(
            "PUT",
            &member_path(&target.user_id),
            &admin.jwt,
            Some(r#"{"role":"superadmin"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(bad_role.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_absent_membership_is_enumeration_safe_404() {
    let (state, store, admin) = create_org_state();
    let target = OrgActor::new_user();
    seed_member(&store, target.principal.clone(), OrgRole::OrgMember).await;

    let response = app::build_app(state, &ApiConfig::for_test())
        .oneshot(mutating(
            "DELETE",
            &member_path(&target.user_id),
            &admin.jwt,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn org_removal_cascades_workspace_grant() {
    let (state, store, admin) = create_org_state();
    let target = OrgActor::new_user();
    seed_member(&store, target.principal.clone(), OrgRole::OrgMember).await;
    let config = ApiConfig::for_test();

    let put = app::build_app(state.clone(), &config)
        .oneshot(mutating(
            "PUT",
            &member_path(&target.user_id),
            &admin.jwt,
            Some(r#"{"role":"viewer"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    let removed = app::build_app(state.clone(), &config)
        .oneshot(mutating(
            "DELETE",
            &format!("/api/v1/orgs/{TEST_ORG}/members/{}", target.user_id),
            &admin.jwt,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::OK);

    let grants = nebula_api::state::MembershipStore::list_workspace_members(
        store.as_ref(),
        TEST_ORG.parse().unwrap(),
        TEST_WS.parse().unwrap(),
    )
    .await
    .unwrap();
    assert!(grants.is_empty());
    let snapshot = nebula_api::state::MembershipStore::get_tenant_membership(
        store.as_ref(),
        TEST_ORG.parse().unwrap(),
        Some(TEST_WS.parse().unwrap()),
        &target.principal,
    )
    .await
    .unwrap();
    assert_eq!(snapshot.org_role, None);
    assert_eq!(snapshot.workspace_role, None);
}
