use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use nebula_api::{WorkerStatus, api_only_app};
use nebula_webhook::{WebhookServer, WebhookServerConfig};
use tower::ServiceExt;

fn test_workers() -> Vec<WorkerStatus> {
    vec![WorkerStatus {
        id: "wrk-1".to_string(),
        status: "idle".to_string(),
        queue_len: 0,
    }]
}

async fn build_app() -> axum::Router {
    let webhook = WebhookServer::new_embedded(WebhookServerConfig::default()).unwrap();
    api_only_app(webhook, test_workers())
}

#[tokio::test]
async fn protected_route_requires_bearer_token() {
    let app = build_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"], "missing_bearer_token");
    assert_eq!(
        payload["message"],
        "Authorization: Bearer <token> is required"
    );
}

#[tokio::test]
async fn issued_oauth_token_grants_access_to_protected_route() {
    let app = build_app().await;

    let oauth_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/oauth/callback")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"provider":"github","code":"mock_test_code","redirectUri":"nebula://auth/callback"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(oauth_response.status(), StatusCode::OK);

    let oauth_body = to_bytes(oauth_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let oauth_payload: serde_json::Value = serde_json::from_slice(&oauth_body).unwrap();
    let access_token = oauth_payload["accessToken"]
        .as_str()
        .expect("oauth callback should return accessToken");

    let me_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/me")
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(me_response.status(), StatusCode::OK);
    let me_body = to_bytes(me_response.into_body(), usize::MAX).await.unwrap();
    let me_payload: serde_json::Value = serde_json::from_slice(&me_body).unwrap();
    assert_eq!(me_payload["provider"], "github");
    assert_eq!(me_payload["accessToken"], access_token);
}
