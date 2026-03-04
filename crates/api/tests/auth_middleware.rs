use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use nebula_api::api_only_app;
use tower::ServiceExt;

async fn build_app() -> axum::Router {
    api_only_app()
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
    assert_eq!(payload["error"], "missing_authentication");
    assert_eq!(
        payload["message"],
        "provide Authorization: Bearer <token> or X-API-Key"
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

#[tokio::test]
async fn protected_route_is_rate_limited_with_retry_after_header() {
    let app = build_app().await;

    let oauth_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/oauth/callback")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"provider":"github","code":"mock_rate_limit_code","redirectUri":"nebula://auth/callback"}"#,
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

    let mut got_rate_limit = None;
    for _ in 0..200 {
        let response = app
            .clone()
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

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            got_rate_limit = Some(response);
            break;
        }
    }

    let response = got_rate_limit.expect("request burst should trigger rate limit");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        response.headers().contains_key(header::RETRY_AFTER),
        "429 must include Retry-After header"
    );
}
