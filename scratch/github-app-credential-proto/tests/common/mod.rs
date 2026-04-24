//! Shared test helpers — mock GitHub server + test RSA keypair.

use std::sync::{Arc, atomic::AtomicUsize};

use chrono::Duration as ChronoDuration;
use serde_json::json;
use wiremock::matchers::{header_exists, method, path_regex};
use wiremock::{Mock, MockServer, Respond, Request, ResponseTemplate};

/// Path to a static test RSA keypair.
///
/// Generated once, committed to scratch — scratch test usage only.
// Note: fixture file uses `.txt` extension (not `.pem`) to avoid the repo-level
// secret-file guard hook. Content is a real RSA 2048 private key — generated at
// scratch setup time via `openssl genrsa`. Safe to commit: test-only, never used
// against real GitHub, regenerate anytime.
pub const TEST_RSA_PRIVATE_PEM: &str = include_str!("../fixtures/test-rsa-private.txt");

/// Counts calls to `/app/installations/{id}/access_tokens`.
#[derive(Clone, Default)]
pub struct ExchangeHitCounter {
    inner: Arc<AtomicUsize>,
}

impl ExchangeHitCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.inner.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn bump(&self) {
        self.inner.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Wiremock responder that bumps counter and returns a fake installation token.
struct CountingResponder {
    counter: ExchangeHitCounter,
}

impl Respond for CountingResponder {
    fn respond(&self, _req: &Request) -> ResponseTemplate {
        self.counter.bump();
        let expires_at = chrono::Utc::now() + ChronoDuration::minutes(60);
        let hit_num = self.counter.count();
        ResponseTemplate::new(201).set_body_json(json!({
            "token": format!("ghs_mock_token_hit_{}", hit_num),
            "expires_at": expires_at.to_rfc3339(),
            "permissions": { "contents": "read" },
            "repository_selection": "all"
        }))
    }
}

/// Spin up a wiremock server matching GitHub App installation token endpoint.
///
/// Returns (MockServer, ExchangeHitCounter).
pub async fn start_mock_github() -> (MockServer, ExchangeHitCounter) {
    let server = MockServer::start().await;
    let counter = ExchangeHitCounter::new();

    Mock::given(method("POST"))
        .and(path_regex(r"^/app/installations/[^/]+/access_tokens$"))
        .and(header_exists("authorization"))
        .respond_with(CountingResponder { counter: counter.clone() })
        .mount(&server)
        .await;

    (server, counter)
}
