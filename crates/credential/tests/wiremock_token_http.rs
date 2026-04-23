//! OAuth2 token JSON over HTTP: [`wiremock`] stands in for a real authorization server.
//! Complements the raw `TcpListener` unit tests in `token_http.rs` (same ADR-0031 policy).

use nebula_credential::credentials::oauth2::token_http::{
    OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES, oauth_token_http_client, read_token_response_limited,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn read_token_response_via_wiremock() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "wiremock-token",
            "token_type": "Bearer",
        })))
        .mount(&server)
        .await;

    let client = oauth_token_http_client();
    let response = client
        .get(format!("{}/token", server.uri()))
        .send()
        .await
        .expect("request to mock token endpoint");

    let value = read_token_response_limited(response, OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES)
        .await
        .expect("bounded read + JSON parse");

    assert_eq!(value["access_token"], "wiremock-token");
    assert_eq!(value["token_type"], "Bearer");
}
