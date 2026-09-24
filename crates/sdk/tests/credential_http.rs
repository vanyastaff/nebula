#![cfg(feature = "http")]

use nebula_sdk::client::{credential::v1::*, http::*};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

enum Reply {
    Bytes(String),
    Disconnect,
    Delay,
}

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let task = tokio::spawn(async move {
            let mut replies = VecDeque::from(replies);
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let size = stream.read(&mut buffer).await.unwrap();
                    if size == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..size]);
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .map(|length| length.parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                match replies.pop_front().unwrap_or(Reply::Disconnect) {
                    Reply::Bytes(bytes) => {
                        let _ = stream.write_all(bytes.as_bytes()).await;
                    },
                    Reply::Disconnect => {},
                    Reply::Delay => tokio::time::sleep(Duration::from_secs(5)).await,
                }
            }
        });
        Self {
            base,
            requests,
            task,
        }
    }

    fn client(&self) -> CredentialClient {
        HttpClient::new(
            &self.base,
            BearerToken::new("bearer-secret-canary").unwrap(),
            HttpOptions::default(),
        )
        .unwrap()
        .credentials("org", "ws")
        .unwrap()
    }

    fn seen(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn response(status: u16, content_type: &str, extra: &str, body: &str) -> Reply {
    Reply::Bytes(format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    ))
}

fn credential() -> String {
    json!({"id":"cred_1","credential_key":"token","name":"Example","description":null,"auth_pattern":"bearer","capabilities":{"interactive":false,"refreshable":false,"testable":false,"revocable":false},"created_at":"2026-09-23T00:00:00Z","updated_at":"2026-09-23T00:00:00Z","expires_at":null,"version":3,"lifecycle":{"status":"ready"},"tags":{}}).to_string()
}

fn create_request() -> CreateCredentialRequest {
    CreateCredentialRequest {
        credential_key: "token".into(),
        name: "Example".into(),
        description: None,
        data: json!({"token":"data-secret-canary"}),
        tags: None,
    }
}

fn update_request() -> UpdateCredentialRequest {
    UpdateCredentialRequest {
        name: None,
        description: None,
        data: Some(json!({"token":"data-secret-canary"})),
        tags: None,
        version: Some(3),
    }
}

#[tokio::test]
async fn exact_success_status_media_type_and_retry_after_grammar_are_enforced() {
    for (status, media) in [(201, "application/json"), (200, "text/html")] {
        let server = Server::start(vec![response(status, media, "", &credential())]).await;
        let error = server.client().create(&create_request()).await.unwrap_err();
        assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
        assert_eq!(error.status(), Some(status));
    }
    for value in ["+5", "0", "-1", "1.5", "18446744073709551616", "garbage"] {
        let server = Server::start(vec![response(
            503,
            "application/problem+json",
            &format!("Retry-After: {value}\r\n"),
            r#"{"type":"about:blank","title":"Unavailable","status":503}"#,
        )])
        .await;
        let error = server.client().get("cred_1").await.unwrap_err();
        assert_eq!(error.kind(), HttpErrorKind::Problem);
        assert_eq!(error.retry_after(), None);
    }
}

#[tokio::test]
async fn crud_sends_exact_wire_contract_without_idempotency_keys() {
    let server = Server::start(vec![
        response(
            200,
            "application/json",
            "",
            r#"{"credentials":[],"total":0,"page":2,"page_size":7}"#,
        ),
        response(200, "application/json", "", &credential()),
        response(200, "application/json", "", &credential()),
        response(200, "application/json", "", &credential()),
        response(200, "application/json", "", r#"{"ok":true}"#),
    ])
    .await;
    let client = server.client();
    assert_eq!(
        client
            .list(&ListCredentialsRequest {
                page: Some(2),
                page_size: Some(7),
                credential_key: Some("a&b=?".into()),
                auth_pattern: None
            })
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(client.create(&create_request()).await.unwrap().id, "cred_1");
    assert_eq!(client.get("cred_1").await.unwrap().id, "cred_1");
    assert_eq!(
        client
            .update("cred_1", &update_request())
            .await
            .unwrap()
            .version,
        3
    );
    assert!(client.delete("cred_1").await.unwrap().ok);
    let requests = server.seen();
    assert_eq!(requests.len(), 5);
    assert!(requests[0].starts_with("GET /api/v1/orgs/org/workspaces/ws/credentials?page=2&page_size=7&credential_key=a%26b%3D%3F HTTP/1.1"));
    for (request, method) in requests.iter().zip(["GET", "POST", "GET", "PUT", "DELETE"]) {
        assert!(request.starts_with(method));
        assert!(request.contains("authorization: Bearer bearer-secret-canary"));
        assert!(!request.to_lowercase().contains("idempotency-key"));
    }
    let body = requests[3].split("\r\n\r\n").nth(1).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).unwrap(),
        json!({"version":3,"data":{"token":"data-secret-canary"}})
    );
}

#[tokio::test]
async fn problem_and_retry_after_are_typed_but_error_formatting_is_redacted() {
    let body = json!({"type":"https://nebula.dev/problems/credential-refresh-not-applied", "title":"server-secret-canary", "status":409, "detail":"server-secret-canary", "extra":"server-secret-canary"}).to_string();
    let server = Server::start(vec![response(
        409,
        "application/problem+json; charset=utf-8",
        "Retry-After: 12\r\n",
        &body,
    )])
    .await;
    let error = server.client().create(&create_request()).await.unwrap_err();
    assert_eq!(error.kind(), HttpErrorKind::Problem);
    assert_eq!(error.status(), Some(409));
    assert_eq!(error.retry_after().unwrap().seconds(), 12);
    assert_eq!(
        error.problem().unwrap().credential_kind(),
        CredentialProblemKind::RefreshNotAppliedAfter
    );
    assert!(!format!("{error:?} {error}").contains("secret-canary"));
    assert!(std::error::Error::source(&error).is_none());
    assert_eq!(server.seen().len(), 1);

    let future = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_mins(1));
    let server = Server::start(vec![response(
        503,
        "application/problem+json",
        &format!("Retry-After: {future}\r\n"),
        r#"{"type":"about:blank","title":"Unavailable","status":503}"#,
    )])
    .await;
    let error = server.client().get("cred_1").await.unwrap_err();
    assert!((1..=60).contains(&error.retry_after().unwrap().seconds()));
}

#[tokio::test]
async fn redirects_and_empty_auth_failures_preserve_status_without_following() {
    let destination = Server::start(vec![]).await;
    let server = Server::start(vec![
        response(
            307,
            "text/plain",
            &format!("Location: {}/stolen\r\n", destination.base),
            "server-secret-canary",
        ),
        response(401, "text/plain", "", ""),
    ])
    .await;
    let client = server.client();
    let error = client.create(&create_request()).await.unwrap_err();
    assert_eq!(error.status(), Some(307));
    assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
    let error = client.get("cred_1").await.unwrap_err();
    assert_eq!(error.status(), Some(401));
    assert_eq!(error.kind(), HttpErrorKind::InvalidResponse);
    assert!(destination.seen().is_empty());
    assert_eq!(server.seen().len(), 2);
}

#[tokio::test]
async fn each_mutation_disconnect_or_invalid_ack_is_unknown_and_never_replayed() {
    for reply in [
        Reply::Disconnect,
        response(200, "application/json", "", "not-json"),
    ] {
        let server = Server::start(vec![reply]).await;
        let error = server.client().create(&create_request()).await.unwrap_err();
        assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
        assert_eq!(server.seen().len(), 1);
    }
    let server = Server::start(vec![
        Reply::Disconnect,
        Reply::Disconnect,
        Reply::Disconnect,
    ])
    .await;
    let client = server.client();
    assert_eq!(
        client
            .update("cred_1", &update_request())
            .await
            .unwrap_err()
            .kind(),
        HttpErrorKind::OutcomeUnknown
    );
    assert_eq!(
        client.delete("cred_1").await.unwrap_err().kind(),
        HttpErrorKind::OutcomeUnknown
    );
    assert_eq!(
        client.get("cred_1").await.unwrap_err().kind(),
        HttpErrorKind::Transport
    );
    assert_eq!(server.seen().len(), 3);
}

#[tokio::test]
async fn response_limit_covers_chunked_bodies_and_total_request_timeout() {
    let server = Server::start(vec![Reply::Bytes("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n10\r\n0123456789abcdef\r\n0\r\n\r\n".into())]).await;
    let client = HttpClient::new(
        &server.base,
        BearerToken::new("token").unwrap(),
        HttpOptions {
            max_response_bytes: 8,
            ..HttpOptions::default()
        },
    )
    .unwrap()
    .credentials("org", "ws")
    .unwrap();
    let error = client.create(&create_request()).await.unwrap_err();
    assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
    assert_eq!(error.status(), Some(200));
    let server = Server::start(vec![Reply::Delay]).await;
    let client = HttpClient::new(
        &server.base,
        BearerToken::new("token").unwrap(),
        HttpOptions {
            request_timeout: Duration::from_millis(50),
            ..HttpOptions::default()
        },
    )
    .unwrap()
    .credentials("org", "ws")
    .unwrap();
    let error = client.delete("cred_1").await.unwrap_err();
    assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
    assert_eq!(server.seen().len(), 1);
}

#[tokio::test]
async fn incomplete_response_body_retains_status_and_unknown_mutation_outcome() {
    let server = Server::start(vec![Reply::Bytes(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{"
            .into(),
    )])
    .await;
    let error = server.client().delete("cred_1").await.unwrap_err();
    assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
    assert_eq!(error.status(), Some(200));
    assert_eq!(server.seen().len(), 1);
}

#[tokio::test]
async fn acquisition_methods_use_exact_paths_bearer_and_public_wire_shapes() {
    let server = Server::start(vec![
        response(200, "application/json", "", r#"{"status":"pending","pending_token":"pending-secret","interaction":{"type":"redirect","url":"https://provider.test/authorize"}}"#),
        response(200, "application/json", "", r#"{"status":"complete","credential_id":"cred_new"}"#),
        response(200, "application/json", "", r#"{"status":"pending","pending_token":"reauth-secret","interaction":{"type":"display_info","title":"Continue","message":"Authorize","data":{},"expires_in":60}}"#),
    ])
    .await;
    let client = server.client();
    let pending = client
        .resolve(&ResolveCredentialRequest {
            credential_key: "oauth2".into(),
            data: json!({"client_secret":"resolve-secret"}),
        })
        .await
        .unwrap();
    assert!(matches!(pending, ResolveCredentialResponse::Pending { .. }));
    let complete = client
        .continue_resolve(&ContinueResolveCredentialRequest {
            credential_key: "oauth2".into(),
            pending_token: "pending-secret".into(),
            user_input: json!("Poll"),
        })
        .await
        .unwrap();
    assert!(
        matches!(complete, ResolveCredentialResponse::Complete { credential_id } if credential_id == "cred_new")
    );
    client
        .reauthorize(
            "cred_existing",
            &ReauthorizeCredentialRequest {
                data: json!({"client_secret":"replacement-secret"}),
            },
        )
        .await
        .unwrap();

    let requests = server.seen();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0].starts_with("POST /api/v1/orgs/org/workspaces/ws/credentials/resolve HTTP/1.1")
    );
    assert!(
        requests[1].starts_with(
            "POST /api/v1/orgs/org/workspaces/ws/credentials/resolve/continue HTTP/1.1"
        )
    );
    assert!(requests[2].starts_with(
        "POST /api/v1/orgs/org/workspaces/ws/credentials/cred_existing/reauthorize HTTP/1.1"
    ));
    for request in &requests {
        assert!(request.contains("authorization: Bearer bearer-secret-canary"));
        assert!(!request.to_lowercase().contains("idempotency-key"));
    }
    let reauthorize_body: serde_json::Value =
        serde_json::from_str(requests[2].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        reauthorize_body,
        json!({"data":{"client_secret":"replacement-secret"}})
    );
}

#[tokio::test]
async fn every_acquisition_failure_is_unknown_and_is_never_replayed() {
    for operation in ["resolve", "continue", "reauthorize"] {
        let server = Server::start(vec![Reply::Disconnect]).await;
        let client = server.client();
        let error = match operation {
            "resolve" => client
                .resolve(&ResolveCredentialRequest {
                    credential_key: "oauth2".into(),
                    data: json!({}),
                })
                .await
                .unwrap_err(),
            "continue" => client
                .continue_resolve(&ContinueResolveCredentialRequest {
                    credential_key: "oauth2".into(),
                    pending_token: "pending-secret".into(),
                    user_input: json!("Poll"),
                })
                .await
                .unwrap_err(),
            "reauthorize" => client
                .reauthorize(
                    "cred_existing",
                    &ReauthorizeCredentialRequest { data: json!({}) },
                )
                .await
                .unwrap_err(),
            _ => unreachable!(),
        };
        assert_eq!(error.kind(), HttpErrorKind::OutcomeUnknown);
        assert_eq!(server.seen().len(), 1);
    }
}

#[tokio::test]
async fn selectors_are_single_segments_and_invalid_configuration_never_sends() {
    let server = Server::start(vec![response(200, "application/json", "", &credential())]).await;
    let http = HttpClient::new(
        &format!("{}/mount/", server.base),
        BearerToken::new("bearer-secret-canary").unwrap(),
        HttpOptions::default(),
    )
    .unwrap();
    let client = http.credentials("https://evil.test/?x", "a/b%2f?").unwrap();
    client.get("id/#?").await.unwrap();
    assert!(server.seen()[0].starts_with("GET /mount/api/v1/orgs/https:%2F%2Fevil.test%2F%3Fx/workspaces/a%2Fb%252f%3F/credentials/id%2F%23%3F HTTP/1.1"));
    for invalid in ["", ".", "..", "bad\n"] {
        assert!(http.credentials(invalid, "ws").is_err());
        assert!(http.credentials("org", invalid).is_err());
        assert!(client.get(invalid).await.is_err());
    }
    for invalid in [
        "file:///tmp",
        "https://user:secret@example.test",
        "https://example.test/?secret",
        "https://example.test/#secret",
    ] {
        let error = HttpClient::new(
            invalid,
            BearerToken::new("token").unwrap(),
            HttpOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), HttpErrorKind::InvalidConfiguration);
        assert!(!format!("{error:?}").contains("secret"));
    }
    assert!(BearerToken::new("bad\r\nheader").is_err());
    assert!(
        !format!(
            "{http:?} {client:?} {:?}",
            BearerToken::new("bearer-secret-canary").unwrap()
        )
        .contains("secret-canary")
    );
    assert_eq!(server.seen().len(), 1);
}
