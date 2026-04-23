//! [`wiremock`] for tests that exercise HTTP **clients** (webhooks, callbacks) without the real
//! network. The full router tests live in `webhook_transport_integration.rs`; this is a minimal
//! harness check.

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn wiremock_returns_json_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ping"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": true, "crate": "nebula-api" })),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let value: serde_json::Value = client
        .get(format!("{}/ping", server.uri()))
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json body");

    assert_eq!(value["ok"], true);
    assert_eq!(value["crate"], "nebula-api");
}
