//! Live compatibility proof: the curated SDK talks to the real API router.
//! Kept in the examples composition member so neither surface depends on the other.

use std::sync::{Arc, Mutex};

use axum::{
    body::{Body, to_bytes},
    middleware,
    response::Response,
};
use nebula_api::{
    ApiConfig, AppState, app,
    error::ApiError,
    state::{OrgResolver, WorkspaceResolver},
};
use nebula_core::{OrgId, OrgRole, Principal, UserId, WorkspaceId};
use nebula_sdk::client::{
    credential::v1::{CreateCredentialRequest, ListCredentialsRequest, UpdateCredentialRequest},
    http::{BearerToken, HttpClient, HttpErrorKind, HttpOptions},
};
use nebula_storage::inmem::{
    InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader, InMemoryNodeResultStore,
    InMemoryStartAcceptanceStore, InMemoryTurnHandoff, InMemoryWorkflowStore,
    InMemoryWorkflowVersionStore,
};
use serde_json::{Value, json};

const ORG: &str = "org_00000000000000000000000001";
const WORKSPACE: &str = "ws_00000000000000000000000001";
const SECRET: &str = "sdk-parity-secret-must-not-appear-in-responses";

struct TenantDirectory;

#[async_trait::async_trait]
impl OrgResolver for TenantDirectory {
    async fn resolve_by_slug(&self, slug: &str) -> Result<OrgId, ApiError> {
        if slug == "sdk-org" {
            Ok(ORG.parse().unwrap())
        } else {
            Err(ApiError::NotFound("not found".into()))
        }
    }
}

#[async_trait::async_trait]
impl WorkspaceResolver for TenantDirectory {
    async fn resolve_by_slug(&self, org: OrgId, slug: &str) -> Result<WorkspaceId, ApiError> {
        if org == ORG && slug == "sdk-workspace" {
            Ok(WORKSPACE.parse().unwrap())
        } else {
            Err(ApiError::NotFound("not found".into()))
        }
    }

    async fn resolve_by_id(
        &self,
        org: OrgId,
        workspace: WorkspaceId,
    ) -> Result<WorkspaceId, ApiError> {
        if org == ORG && workspace == WORKSPACE {
            Ok(workspace)
        } else {
            Err(ApiError::NotFound("not found".into()))
        }
    }
}

struct Observation {
    method: String,
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

struct Server {
    base: String,
    bearer: String,
    observations: Arc<Mutex<Vec<Observation>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start() -> Self {
        let config = ApiConfig::for_test();
        let user = UserId::new();
        let now = chrono::Utc::now().timestamp() as u64;
        let bearer = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &nebula_api::middleware::auth::Claims {
                sub: user.to_string(),
                iat: now,
                exp: now + 3600,
            },
            &jsonwebtoken::EncodingKey::from_secret(config.jwt_secret.as_bytes()),
        )
        .unwrap();
        let memberships =
            Arc::new(nebula_api::domain::org::membership::InMemoryMembershipStore::new());
        memberships
            .seed_for_test(
                ORG.parse().unwrap(),
                Principal::User(user),
                OrgRole::OrgOwner,
            )
            .await;
        let executions = InMemoryExecutionStore::new();
        let versions = InMemoryWorkflowVersionStore::new();
        let workflows = InMemoryWorkflowStore::new_with_versions(&versions, &executions);
        let state = AppState::new(
            Arc::new(workflows),
            Arc::new(versions),
            Arc::new(executions.clone()),
            Arc::new(InMemoryNodeResultStore::new()),
            Arc::new(InMemoryJournalReader::new(&executions)),
            Arc::new(InMemoryControlQueue::new(&executions)),
            Arc::new(InMemoryStartAcceptanceStore::new(&executions)),
            Arc::new(InMemoryTurnHandoff::new(&executions)),
            Arc::new(InMemoryStartAcceptanceStore::new(&executions)),
            config.jwt_secret.clone(),
        )
        .with_org_resolver(Arc::new(TenantDirectory))
        .with_workspace_resolver(Arc::new(TenantDirectory))
        .with_membership_store(memberships);
        let key = Arc::new(
            nebula_storage::credential::EnvKeyProvider::from_base64(
                "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=",
            )
            .unwrap(),
        );
        let service = nebula_api::ports::credential_service_factory::with_memory_store(key)
            .await
            .unwrap();
        let state = state.with_credential_gateway(
            nebula_api::ports::credential_command::test_gateway_from_service(service),
        );
        let observations = Arc::new(Mutex::new(Vec::new()));
        let capture = observations.clone();
        let router = app::build_app(state, &config).layer(middleware::from_fn(
            move |request: axum::extract::Request, next: middleware::Next| {
                let capture = capture.clone();
                async move {
                    assert!(request.headers().contains_key("authorization"));
                    assert!(!request.headers().contains_key("cookie"));
                    assert!(!request.headers().contains_key("x-csrf-token"));
                    let method = request.method().to_string();
                    let response = next.run(request).await;
                    let (parts, body) = response.into_parts();
                    let body = to_bytes(body, 1024 * 1024).await.unwrap();
                    capture.lock().unwrap().push(Observation {
                        method,
                        status: parts.status.as_u16(),
                        content_type: parts
                            .headers
                            .get("content-type")
                            .map(|v| v.to_str().unwrap().to_owned()),
                        body: body.to_vec(),
                    });
                    Response::from_parts(parts, Body::from(body))
                }
            },
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            base,
            bearer,
            observations,
            task,
        }
    }

    fn client(&self, bearer: &str) -> nebula_sdk::client::http::CredentialClient {
        HttpClient::new(
            &self.base,
            BearerToken::new(bearer).unwrap(),
            HttpOptions::default(),
        )
        .unwrap()
        .credentials("sdk-org", "sdk-workspace")
        .unwrap()
    }
}

#[tokio::test]
async fn sdk_crud_matches_live_api_methods_statuses_and_bearer_authority() {
    let server = Server::start().await;
    let client = server.client(&server.bearer);
    let empty = client
        .list(&ListCredentialsRequest::default())
        .await
        .unwrap();
    assert_eq!((empty.total, empty.page, empty.page_size), (0, 1, 20));
    let created = client
        .create(&CreateCredentialRequest {
            credential_key: "api_key".into(),
            name: "SDK parity".into(),
            description: None,
            data: json!({"api_key": SECRET}),
            tags: None,
        })
        .await
        .unwrap();
    assert_eq!(created.credential_key, "api_key");
    assert_eq!(client.get(&created.id).await.unwrap().name, "SDK parity");
    let listed = client
        .list(&ListCredentialsRequest {
            page: Some(1),
            page_size: Some(1),
            credential_key: Some("api_key".into()),
            auth_pattern: Some(created.auth_pattern.clone()),
        })
        .await
        .unwrap();
    assert_eq!((listed.total, listed.page, listed.page_size), (1, 1, 1));
    assert_eq!(listed.credentials[0].id, created.id);
    let update = UpdateCredentialRequest {
        name: Some("SDK renamed".into()),
        description: None,
        data: None,
        tags: None,
        version: Some(created.version),
    };
    let updated = client.update(&created.id, &update).await.unwrap();
    assert!(updated.version > created.version);
    assert_eq!(client.get(&created.id).await.unwrap().name, "SDK renamed");
    let conflict = client.update(&created.id, &update).await.unwrap_err();
    assert_eq!(conflict.kind(), HttpErrorKind::Problem);
    assert_eq!(conflict.status(), Some(409));
    assert_eq!(conflict.problem().unwrap().problem.status, 409);
    assert!(client.delete(&created.id).await.unwrap().ok);
    let absent = client.get(&created.id).await.unwrap_err();
    assert_eq!(absent.kind(), HttpErrorKind::Problem);
    assert_eq!(absent.status(), Some(404));
    assert_eq!(
        client
            .list(&ListCredentialsRequest::default())
            .await
            .unwrap()
            .total,
        0
    );

    let observations = server.observations.lock().unwrap();
    assert_eq!(
        observations
            .iter()
            .map(|o| (o.method.as_str(), o.status))
            .collect::<Vec<_>>(),
        [
            ("GET", 200),
            ("POST", 200),
            ("GET", 200),
            ("GET", 200),
            ("PUT", 200),
            ("GET", 200),
            ("PUT", 409),
            ("DELETE", 200),
            ("GET", 404),
            ("GET", 200)
        ]
    );
    for observation in observations.iter() {
        assert!(!String::from_utf8_lossy(&observation.body).contains(SECRET));
        let body: Value = serde_json::from_slice(&observation.body).unwrap();
        assert!(body.get("data").is_none());
        if observation.status >= 400 {
            assert_eq!(
                observation.content_type.as_deref(),
                Some("application/problem+json")
            );
            assert_eq!(body["status"], observation.status);
        }
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&observations[7].body).unwrap(),
        json!({"ok": true})
    );
}

#[tokio::test]
async fn real_empty_auth_failure_retains_status_without_replaying_mutation() {
    let server = Server::start().await;
    let client = server.client("invalid-bearer");
    let error = client
        .create(&CreateCredentialRequest {
            credential_key: "api_key".into(),
            name: "Rejected".into(),
            description: None,
            data: json!({"api_key": SECRET}),
            tags: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
    assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
    assert!(error.problem().is_none());
    assert_eq!(
        server
            .client(&server.bearer)
            .list(&ListCredentialsRequest::default())
            .await
            .unwrap()
            .total,
        0
    );
    let observations = server.observations.lock().unwrap();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].method, "POST");
    assert_eq!(observations[0].status, 401);
    assert!(observations[0].body.is_empty());
}
