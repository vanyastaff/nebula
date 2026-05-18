//! Runtime access-control E2E tests.
//!
//! These tests prove tenant RBAC and PAT grants are both enforced at request
//! time. The route metadata/OpenAPI coverage tests live elsewhere; this file
//! drives real HTTP requests through the app router.

mod common;

use std::{str::FromStr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
};
use nebula_api::{
    ApiConfig, AppState, app,
    domain::auth::backend::{
        AuthBackend, CreatePatParams, InMemoryAuthBackend, SignupRequest, dto::SecretString,
    },
    error::ApiError,
    state::{AddMemberOutcome, MembershipStore, OrgMember, RemoveMemberOutcome},
};
use nebula_core::{OrgId, OrgRole, Principal, UserId, WorkspaceId, WorkspaceRole};
use tower::ServiceExt;

use common::{
    TEST_ORG, TEST_WS, build_me_state, create_state_with_queue, me_support::jwt_for, ws_path,
};

#[derive(Clone)]
struct FixedMembershipStore {
    principal: Principal,
    org_role: OrgRole,
    workspace_role: WorkspaceRole,
}

#[async_trait::async_trait]
impl MembershipStore for FixedMembershipStore {
    async fn get_org_role(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<Option<OrgRole>, ApiError> {
        if org_id == test_org_id() && principal == &self.principal {
            Ok(Some(self.org_role))
        } else {
            Ok(None)
        }
    }

    async fn get_workspace_role(
        &self,
        workspace_id: WorkspaceId,
        principal: &Principal,
    ) -> Result<Option<WorkspaceRole>, ApiError> {
        if workspace_id == test_ws_id() && principal == &self.principal {
            Ok(Some(self.workspace_role))
        } else {
            Ok(None)
        }
    }

    async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>, ApiError> {
        if org_id == test_org_id() {
            Ok(vec![OrgMember {
                principal: self.principal.clone(),
                role: self.org_role,
            }])
        } else {
            Ok(Vec::new())
        }
    }

    async fn add_member(
        &self,
        _org_id: OrgId,
        _principal: &Principal,
        _role: OrgRole,
    ) -> Result<(), ApiError> {
        Ok(())
    }

    async fn remove_member(
        &self,
        _org_id: OrgId,
        _principal: &Principal,
    ) -> Result<bool, ApiError> {
        Ok(false)
    }

    async fn add_member_guarded(
        &self,
        _org_id: OrgId,
        _principal: &Principal,
        _role: OrgRole,
    ) -> Result<AddMemberOutcome, ApiError> {
        Ok(AddMemberOutcome::Added)
    }

    async fn remove_member_guarded(
        &self,
        _org_id: OrgId,
        _principal: &Principal,
    ) -> Result<RemoveMemberOutcome, ApiError> {
        Ok(RemoveMemberOutcome::NotFound)
    }

    async fn list_orgs_for_principal(
        &self,
        principal: &Principal,
    ) -> Result<Vec<(OrgId, OrgRole)>, ApiError> {
        if principal == &self.principal {
            Ok(vec![(test_org_id(), self.org_role)])
        } else {
            Ok(Vec::new())
        }
    }
}

fn test_org_id() -> OrgId {
    TEST_ORG.parse().expect("TEST_ORG must be a valid OrgId")
}

fn test_ws_id() -> WorkspaceId {
    TEST_WS
        .parse()
        .expect("TEST_WS must be a valid WorkspaceId")
}

async fn state_with_pat_and_workspace_role(
    scopes: Vec<&str>,
    workspace_role: WorkspaceRole,
) -> (AppState, String) {
    let backend = Arc::new(InMemoryAuthBackend::new());
    let profile = backend
        .register_user(SignupRequest {
            email: "access-e2e@nebula.dev".to_owned(),
            password: SecretString::new("hunter22".to_owned()),
            display_name: "Access E2E".to_owned(),
        })
        .await
        .expect("register access e2e user");
    let user_id = UserId::from_str(&profile.user_id).expect("registered user id parses");
    let minted = backend
        .create_pat(
            &profile.user_id,
            CreatePatParams {
                name: "access-e2e".to_owned(),
                scopes: scopes.into_iter().map(str::to_owned).collect(),
                ttl_seconds: None,
            },
        )
        .await
        .expect("mint access e2e PAT");

    let backend_dyn: Arc<dyn AuthBackend> = backend;
    let membership_dyn: Arc<dyn MembershipStore> = Arc::new(FixedMembershipStore {
        principal: Principal::User(user_id),
        org_role: OrgRole::OrgMember,
        workspace_role,
    });
    let state = build_me_state()
        .with_auth_backend(backend_dyn)
        .with_membership_store(membership_dyn);

    (state, minted.plaintext)
}

async fn post_workflows_with_pat(state: AppState, pat: &str) -> Response<Body> {
    let app = app::build_app(state, &ApiConfig::for_test());
    let create_request = serde_json::json!({
        "name": "Access E2E Workflow",
        "description": "Created by access runtime test",
        "definition": {
            "nodes": [],
            "edges": []
        }
    });

    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(ws_path("/workflows"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {pat}"))
            .body(Body::from(serde_json::to_string(&create_request).unwrap()))
            .unwrap(),
    )
    .await
    .expect("app must respond")
}

#[tokio::test]
async fn editor_pat_without_write_scope_cannot_create_workflow() {
    let (state, pat) =
        state_with_pat_and_workspace_role(vec!["workflows:read"], WorkspaceRole::WorkspaceEditor)
            .await;

    let response = post_workflows_with_pat(state, &pat).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn viewer_full_access_pat_cannot_create_workflow() {
    let (state, pat) =
        state_with_pat_and_workspace_role(vec!["full_access"], WorkspaceRole::WorkspaceViewer)
            .await;

    let response = post_workflows_with_pat(state, &pat).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn editor_full_access_pat_can_create_workflow() {
    let (state, pat) =
        state_with_pat_and_workspace_role(vec!["full_access"], WorkspaceRole::WorkspaceEditor)
            .await;

    let response = post_workflows_with_pat(state, &pat).await;

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn editor_pat_with_write_scope_reaches_workflow_handler() {
    let (state, pat) =
        state_with_pat_and_workspace_role(vec!["workflows:write"], WorkspaceRole::WorkspaceEditor)
            .await;

    let response = post_workflows_with_pat(state, &pat).await;

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn jwt_bypass_harness_without_membership_store_can_still_list_workflows() {
    let (state, _queue) = create_state_with_queue().await;
    let jwt = jwt_for(&UserId::new().to_string());
    let app = app::build_app(state, &ApiConfig::for_test());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/workflows"))
                .header("authorization", format!("Bearer {jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn configured_membership_store_with_no_roles_denies_tenant_access() {
    let request_user_id = UserId::new();
    let seeded_other_user_id = UserId::new();
    let membership_dyn: Arc<dyn MembershipStore> = Arc::new(FixedMembershipStore {
        principal: Principal::User(seeded_other_user_id),
        org_role: OrgRole::OrgMember,
        workspace_role: WorkspaceRole::WorkspaceViewer,
    });
    let (state, _queue) = create_state_with_queue().await;
    let state = state.with_membership_store(membership_dyn);
    let app = app::build_app(state, &ApiConfig::for_test());
    let jwt = jwt_for(&request_user_id.to_string());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/workflows"))
                .header("authorization", format!("Bearer {jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
