//! Integration contract for route-scoped internal authentication.

mod common;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::create_state_with_queue;
use nebula_api::{ApiConfig, app, middleware::X_INTERNAL_TOKEN};
use tower::ServiceExt;

const RELOAD_PATH: &str = "/internal/v1/webhooks/reload";
const INTERNAL_TOKEN: &str = "integration-test-internal-token";

async fn call_internal(
    app: Router,
    path: &str,
    presented_token: Option<&str>,
) -> (StatusCode, String) {
    let mut request = Request::builder().method(Method::POST).uri(path);
    if let Some(token) = presented_token {
        request = request.header(X_INTERNAL_TOKEN, token);
    }

    let response = app
        .oneshot(request.body(Body::empty()).expect("valid internal request"))
        .await
        .expect("internal route response");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read internal route response body");
    let body = String::from_utf8(body.to_vec()).expect("internal route response is UTF-8");

    (status, body)
}

#[tokio::test]
async fn internal_auth_is_fail_closed_without_masking_absent_routes() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app_without_token = app::build_app(state.clone(), &config);

    let absent = call_internal(
        app_without_token.clone(),
        "/internal/v1/webhooks/not-a-route",
        None,
    )
    .await;
    assert_eq!(
        absent.0,
        StatusCode::NOT_FOUND,
        "internal auth must not replace the application fallback for an absent path; body={}",
        absent.1
    );

    let unconfigured = call_internal(app_without_token, RELOAD_PATH, None).await;
    assert_eq!(
        unconfigured,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "internal auth not configured".to_owned(),
        ),
        "the real reload endpoint must fail closed before its handler when no token is configured"
    );

    let app_with_token = app::build_app(state.with_internal_shared_token(INTERNAL_TOKEN), &config);

    let missing = call_internal(app_with_token.clone(), RELOAD_PATH, None).await;
    assert_eq!(
        missing,
        (
            StatusCode::UNAUTHORIZED,
            "missing X-Internal-Token".to_owned(),
        ),
        "a configured internal endpoint must reject a missing header"
    );

    let wrong = call_internal(app_with_token.clone(), RELOAD_PATH, Some("wrong-token")).await;
    assert_eq!(
        wrong,
        (
            StatusCode::UNAUTHORIZED,
            "invalid X-Internal-Token".to_owned(),
        ),
        "a configured internal endpoint must reject a mismatched header"
    );

    let authorized = call_internal(app_with_token, RELOAD_PATH, Some(INTERNAL_TOKEN)).await;
    assert_eq!(
        authorized,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook activation store not configured".to_owned(),
        ),
        "the correct token must reach the handler, which reports its unwired store honestly"
    );
}
