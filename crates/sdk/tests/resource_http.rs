//! The HTTP resource adapter against a raw TCP server, through a real
//! `Manager`: redirects, secrecy, the sent-state table, throttles, retries
//! within a unit, credentials and streamed bodies.
//!
//! Every server behaviour is scripted ([`Reply`]) and observed through its
//! counters (requests read, connections accepted, disconnects, bytes
//! written); the only waits are the ones under test — a request timeout, a
//! unit deadline, a provider's `Retry-After`.

#![cfg(feature = "resource-http")]

#[path = "support/raw_http.rs"]
mod raw_http;

use std::{
    fmt,
    num::NonZeroU32,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use nebula_resource::{
    Manager, RegistrationSpec, ResidentConfig, ScopeLevel, ShutdownConfig, SlotIdentity,
    rate_limit::RowLimit,
};
use nebula_sdk::integration::resource::{
    CredentialGuard, CredentialUnavailableReason, Effect, Error, ErrorKind, HasCredentialSlots,
    OperationError, PinSlots, Provider, Rate, Resident, ResidentProvider, ResourceConfig,
    ResourceContext, ResourceHandle, ResourceKey, ResourceMetadataDraft, SentState, SlotCell,
    http::{
        Authorize, HttpApi, HttpConfig, HttpTransport, Request, open_stream, open_stream_until,
    },
    resource_key,
};
use nebula_sdk::prelude::{IdentityPassword, SecretString, SecretToken};
use raw_http::{Reply, Server};
use tokio_util::sync::CancellationToken;

// ── the resource ─────────────────────────────────────────────────────────

/// How the test resource authenticates.
#[derive(Clone, Copy, Debug)]
enum Auth {
    Bearer,
    Basic,
    ApiKey,
    Anonymous,
}

/// An HTTP API resource whose slots the test keeps a handle on, so it can
/// bind and rotate them after registration.
#[derive(Clone)]
struct Api {
    auth: Auth,
    token: Arc<SlotCell<CredentialGuard<SecretToken>>>,
    login: Arc<SlotCell<CredentialGuard<IdentityPassword>>>,
    creates: Arc<AtomicUsize>,
}

struct Pinned {
    auth: Auth,
    token: Option<Arc<CredentialGuard<SecretToken>>>,
    login: Option<Arc<CredentialGuard<IdentityPassword>>>,
}

impl HasCredentialSlots for Api {
    fn credential_slot_epoch(&self) -> u64 {
        self.token
            .generation()
            .wrapping_mul(31)
            .wrapping_add(self.login.generation())
    }
    fn declares_credential_slots() -> bool {
        true
    }
    fn credential_slot_names() -> &'static [&'static str] {
        &["token", "login"]
    }
}

impl PinSlots for Api {
    type Pinned = Pinned;

    fn pin_slots(&self) -> Pinned {
        Pinned {
            auth: self.auth,
            token: self.token.load(),
            login: self.login.load(),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Api {
    type Config = HttpConfig;
    type Instance = HttpTransport;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.http-api")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_sdk::prelude::metadata_name!("HTTP API"),
            "",
        )
    }

    async fn create(
        &self,
        config: &HttpConfig,
        _: &ResourceContext,
    ) -> Result<HttpTransport, Error> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        HttpTransport::new(config)
    }
}

impl ResidentProvider for Api {}

impl HttpApi for Api {
    fn authorize(slots: &Pinned, auth: &mut Authorize<'_>) -> Result<(), OperationError> {
        match slots.auth {
            Auth::Bearer => auth.bearer(slots.token.as_deref()),
            Auth::Basic => auth.basic(slots.login.as_deref()),
            Auth::ApiKey => auth.api_key_header("x-api-key", slots.token.as_deref()),
            Auth::Anonymous => auth.none(),
        }
    }
}

// ── harness ──────────────────────────────────────────────────────────────

const TOKEN: &str = "tok-canary-7f3e";

fn token(value: &str) -> Arc<CredentialGuard<SecretToken>> {
    Arc::new(CredentialGuard::new(SecretToken::new(SecretString::new(
        value,
    ))))
}

struct Harness {
    manager: Manager,
    api: Api,
}

impl Harness {
    fn new(auth: Auth, config: HttpConfig, limit: Option<RowLimit>) -> Self {
        let api = Api {
            auth,
            token: Arc::new(SlotCell::empty()),
            login: Arc::new(SlotCell::empty()),
            creates: Arc::default(),
        };
        let manager = Manager::new();
        manager
            .register(RegistrationSpec {
                resource: api.clone(),
                config,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: limit,
            })
            .expect("register");
        Self { manager, api }
    }

    /// A bearer-authenticated resource over `server`, its token bound.
    fn bearer(server: &Server) -> Self {
        let harness = Self::new(Auth::Bearer, HttpConfig::new(&server.base), None);
        harness.api.token.store(token(TOKEN));
        harness
    }

    /// The row's resource handle, as an action would hold it: each unit
    /// checks out the instance per attempt.
    fn managed(&self) -> ResourceHandle<Api> {
        let context = ResourceContext::minimal(Default::default(), CancellationToken::new());
        self.manager.handle::<Api>(&context).expect("handle")
    }
}

fn ok(body: &str) -> Reply {
    Reply::Bytes(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ))
}

fn status(code: u16, extra: &str) -> Reply {
    Reply::Bytes(format!(
        "HTTP/1.1 {code} Test\r\nContent-Length: 0\r\n{extra}\r\n"
    ))
}

fn assert_unit_error(error: &OperationError, kind: &ErrorKind, sent: SentState) {
    assert_eq!(error.kind(), kind, "{error:?}");
    assert_eq!(error.sent(), sent, "{error:?}");
}

// ── redirects ────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_redirect_is_an_answer_and_is_never_followed() {
    let destination = Server::start(vec![]).await;
    let redirect = || {
        Reply::Bytes(format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {}/stolen\r\nContent-Length: 0\r\n\r\n",
            destination.base
        ))
    };
    let server = Server::start(vec![redirect(), redirect(), redirect()]).await;

    let bearer = Harness::bearer(&server);
    let managed = bearer.managed();
    let got = managed
        .submit(Request::get("/user").expect("path"))
        .await
        .expect("a 307 is an answer");
    assert_eq!(got.status().as_u16(), 307);
    let posted = managed
        .submit(
            Request::post("/issues")
                .expect("path")
                .json(&serde_json::json!({ "title": "body-canary" }))
                .expect("json"),
        )
        .await
        .expect("a 307 is an answer");
    assert_eq!(posted.status().as_u16(), 307);

    let keyed = Harness::new(Auth::ApiKey, HttpConfig::new(&server.base), None);
    keyed.api.token.store(token("key-canary"));
    let got = keyed
        .managed()
        .submit(Request::get("/user").expect("path"))
        .await
        .expect("a 307 is an answer");
    assert_eq!(got.status().as_u16(), 307);

    assert_eq!(destination.accepts(), 0, "the credential never left");
    assert!(destination.seen().is_empty());
    let seen = server.seen();
    assert_eq!(seen.len(), 3);
    assert!(
        seen[1].contains("body-canary"),
        "the body went to the origin"
    );
    assert!(seen[2].contains("x-api-key: key-canary"));
}

// ── secrecy ──────────────────────────────────────────────────────────────

/// Records every span and event field, whatever its level.
#[derive(Clone, Default)]
struct Capture {
    lines: Arc<Mutex<Vec<String>>>,
    ids: Arc<AtomicU64>,
}

impl Capture {
    fn text(&self) -> String {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .join("\n")
    }

    fn push(&self, line: String) {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }
}

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.0.push_str(&format!(" {}={value:?}", field.name()));
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut line = format!("span {}", attributes.metadata().name());
        attributes.record(&mut Fields(&mut line));
        self.push(line);
        tracing::span::Id::from_u64(self.ids.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut line = String::from("record");
        values.record(&mut Fields(&mut line));
        self.push(line);
    }

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = format!("event {}", event.metadata().target());
        event.record(&mut Fields(&mut line));
        self.push(line);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn credentials_urls_and_bodies_never_reach_debug_or_logs() {
    let capture = Capture::default();
    let _default = tracing::subscriber::set_default(capture.clone());
    let server = Server::start(vec![
        ok(r#"{"login":"body-canary"}"#),
        status(401, "WWW-Authenticate: Bearer realm=\"realm-canary\"\r\n"),
    ])
    .await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let request = || {
        Request::get("/path-canary")
            .expect("path")
            .query(&[("q", "query-canary")])
    };

    let unit = managed.submit(request());
    let mut debug = vec![format!("{:?}", request()), format!("{unit:?}")];
    let response = unit.await.expect("200");
    debug.push(format!("{response:?}"));
    let refused = managed.submit(request()).await.expect_err("401 is refused");
    assert_unit_error(&refused, &ErrorKind::Permanent, SentState::Sent);
    assert_eq!(refused.detail(), "Unauthorized");
    debug.push(format!("{refused:?}"));
    debug.push(refused.to_string());
    debug.push(Error::from(refused).to_string());
    debug.push(format!(
        "{:?}",
        HttpTransport::new(&HttpConfig::new(&server.base)).expect("transport")
    ));

    let seen = server.seen();
    assert!(seen[0].contains(&format!("authorization: Bearer {TOKEN}")));
    assert!(seen[0].starts_with("GET /path-canary?q=query-canary HTTP/1.1"));
    let logs = capture.text();
    assert!(logs.contains("nebula.sdk.http.attempt"), "{logs}");
    assert!(logs.contains("nebula.resource.unit"), "{logs}");
    let port = server.base.rsplit(':').next().expect("port");
    for text in debug.iter().chain([&logs]) {
        for canary in [
            TOKEN,
            "path-canary",
            "query-canary",
            "body-canary",
            "realm-canary",
        ] {
            assert!(!text.contains(canary), "`{canary}` leaked: {text}");
        }
        // The connection pool's own trace events name the origin (never a
        // path, query or credential, as the canaries above show); nothing
        // Nebula emits names it.
        let address = format!("127.0.0.1:{port}");
        let leaked: Vec<&str> = text
            .lines()
            .filter(|line| line.contains(&address) && !line.starts_with("event hyper_util"))
            .collect();
        assert!(leaked.is_empty(), "the URL leaked: {leaked:#?}");
    }
}

// ── sent state and effect ────────────────────────────────────────────────

struct SentCase {
    reply: Option<Reply>,
    post: bool,
    sent: SentState,
    retryable: bool,
    requests: usize,
}

/// Without a reply the request targets a port nothing listens on, so
/// connecting is refused.
async fn sent_case(case: SentCase) {
    let refused = case.reply.is_none();
    let server = Server::start(case.reply.into_iter().collect()).await;
    let target = if refused {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        format!("http://{}", listener.local_addr().expect("addr"))
    } else {
        server.base.clone()
    };
    // A refused connect is reported by the connector (on some platforms
    // only after its own retries); a request timeout firing first could not
    // tell it from a lost request and would be `MaybeSent`.
    let config = if refused {
        HttpConfig::new(target)
    } else {
        HttpConfig::new(target).with_request_timeout(Duration::from_millis(300))
    };
    let harness = Harness::new(Auth::Bearer, config, None);
    harness.api.token.store(token(TOKEN));
    let managed = harness.managed();
    let result = if case.post {
        managed
            .submit(Request::post("/charges").expect("path").body("{}"))
            .await
    } else {
        managed
            .submit(Request::get("/charges").expect("path"))
            .await
    };
    let error = result.expect_err("the exchange failed");
    assert_unit_error(&error, &ErrorKind::Transient, case.sent);
    assert_eq!(error.is_retryable(), case.retryable, "{error:?}");
    assert_eq!(
        error.effect(),
        if case.post {
            Effect::Write
        } else {
            Effect::Read
        }
    );
    if !case.retryable {
        assert_eq!(*Error::from(error).kind(), ErrorKind::OutcomeUnknown);
    }
    assert_eq!(server.seen().len(), case.requests);
}

#[tokio::test]
async fn a_refused_connection_is_not_sent_and_retryable_even_for_a_write() {
    sent_case(SentCase {
        reply: None,
        post: true,
        sent: SentState::NotSent,
        retryable: true,
        requests: 0,
    })
    .await;
}

#[tokio::test]
async fn a_connection_closed_before_the_request_is_maybe_sent() {
    sent_case(SentCase {
        reply: Some(Reply::AcceptThenClose),
        post: true,
        sent: SentState::MaybeSent,
        retryable: false,
        requests: 0,
    })
    .await;
}

#[tokio::test]
async fn no_head_before_the_timeout_is_maybe_sent_unknown_for_a_write_retryable_for_a_read() {
    for post in [true, false] {
        sent_case(SentCase {
            reply: Some(Reply::Hang),
            post,
            sent: SentState::MaybeSent,
            retryable: !post,
            requests: 1,
        })
        .await;
    }
}

#[tokio::test]
async fn a_head_without_its_body_is_interrupted_and_unknown_for_a_write() {
    sent_case(SentCase {
        reply: Some(Reply::HeadersThenHang(
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n".to_owned(),
        )),
        post: true,
        sent: SentState::MaybeSent,
        retryable: false,
        requests: 1,
    })
    .await;
}

// ── throttles and server errors ──────────────────────────────────────────

#[tokio::test]
async fn a_throttle_is_exhausted_retryable_and_pauses_the_next_unit() {
    let server = Server::start(vec![status(429, "Retry-After: 2\r\n"), ok("{}")]).await;
    let rate = Rate::per_second(NonZeroU32::new(100).expect("non-zero"))
        .with_burst(NonZeroU32::new(100).expect("non-zero"))
        .expect("rate");
    let harness = Harness::new(
        Auth::Bearer,
        HttpConfig::new(&server.base),
        Some(RowLimit::rate(rate)),
    );
    harness.api.token.store(token(TOKEN));
    let managed = harness.managed();

    let error = managed
        .submit(Request::post("/messages").expect("path").body("hi"))
        .await
        .expect_err("throttled");
    assert_unit_error(
        &error,
        &ErrorKind::Exhausted {
            retry_after: Some(Duration::from_secs(2)),
        },
        SentState::Sent,
    );
    assert!(error.is_retryable(), "the provider applied nothing");
    assert_eq!(error.retry_after(), Some(Duration::from_secs(2)));

    let started = Instant::now();
    managed
        .submit(Request::get("/messages").expect("path"))
        .await
        .expect("after the pause");
    assert!(
        started.elapsed() >= Duration::from_millis(1500),
        "the next unit waited on the limiter's pause: {:?}",
        started.elapsed()
    );
    assert_eq!(server.seen().len(), 2);
}

#[tokio::test]
async fn a_503_without_retry_after_is_interrupted() {
    let server = Server::start(vec![status(503, "")]).await;
    let harness = Harness::bearer(&server);
    let error = harness
        .managed()
        .submit(Request::get("/status").expect("path"))
        .await
        .expect_err("unavailable");
    assert_unit_error(&error, &ErrorKind::Transient, SentState::MaybeSent);
    assert!(error.is_retryable(), "a read");
}

#[tokio::test]
async fn a_throttled_write_is_re_attempted_after_the_pause_and_a_rejection_is_not() {
    let two = NonZeroU32::new(2).expect("two");
    let rate = Rate::per_second(NonZeroU32::new(100).expect("non-zero"))
        .with_burst(NonZeroU32::new(100).expect("non-zero"))
        .expect("rate");

    let server = Server::start(vec![status(429, "Retry-After: 1\r\n"), ok("{}")]).await;
    let harness = Harness::new(
        Auth::Bearer,
        HttpConfig::new(&server.base),
        Some(RowLimit::rate(rate)),
    );
    harness.api.token.store(token(TOKEN));
    let started = Instant::now();
    let response = harness
        .managed()
        .submit(
            Request::post("/messages")
                .expect("path")
                .body("hi")
                .max_attempts(two),
        )
        .await
        .expect("the provider applied nothing: the write is sent again");
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(server.seen().len(), 2);
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "the second attempt's booking waited out the pause: {:?}",
        started.elapsed()
    );

    let server = Server::start(vec![status(422, ""), ok("{}")]).await;
    let harness = Harness::bearer(&server);
    let error = harness
        .managed()
        .submit(Request::get("/items").expect("path").max_attempts(two))
        .await
        .expect_err("rejected");
    assert_unit_error(&error, &ErrorKind::Permanent, SentState::Sent);
    assert_eq!(server.seen().len(), 1, "a rejection is never re-attempted");
}

// ── attempts within a unit ───────────────────────────────────────────────

#[tokio::test]
async fn a_read_is_re_attempted_after_a_reset_and_a_write_is_not() {
    let two = NonZeroU32::new(2).expect("two");

    let server = Server::start(vec![Reply::Disconnect, ok("{}")]).await;
    let harness = Harness::bearer(&server);
    let response = harness
        .managed()
        .submit(Request::get("/items").expect("path").max_attempts(two))
        .await
        .expect("the second attempt answered");
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(server.seen().len(), 2);

    let server = Server::start(vec![Reply::Disconnect, ok("{}")]).await;
    let harness = Harness::bearer(&server);
    let error = harness
        .managed()
        .submit(
            Request::post("/items")
                .expect("path")
                .body("{}")
                .max_attempts(two),
        )
        .await
        .expect_err("a write is never re-attempted after it may have been sent");
    assert_unit_error(&error, &ErrorKind::Transient, SentState::MaybeSent);
    assert_eq!(server.seen().len(), 1);

    let server = Server::start(vec![Reply::Disconnect, ok("{}")]).await;
    let harness = Harness::bearer(&server);
    harness
        .managed()
        .submit(
            Request::post("/items")
                .expect("path")
                .body("{}")
                .idempotency_key("key-1")
                .max_attempts(two),
        )
        .await
        .expect("a keyed write is replay safe");
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    let keys: Vec<&str> = seen
        .iter()
        .map(|request| idempotency_header(request))
        .collect();
    // The header is the key the unit derived from the developer part
    // (base64url SHA-256), never the part itself, and every attempt sends
    // the same one.
    assert_eq!(keys[0].len(), 43, "{keys:?}");
    assert!(
        keys[0]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
        "{keys:?}"
    );
    assert_ne!(keys[0], "key-1");
    assert_eq!(keys[0], keys[1], "one key for every attempt");

    // The same part derives the same key; another part another one.
    let server = Server::start(vec![ok("{}"), ok("{}")]).await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    for part in ["key-1", "key-2"] {
        managed
            .submit(
                Request::post("/items")
                    .expect("path")
                    .body("{}")
                    .idempotency_key(part),
            )
            .await
            .expect("keyed");
    }
    let seen = server.seen();
    assert_eq!(idempotency_header(&seen[0]), keys[0]);
    assert_ne!(idempotency_header(&seen[1]), keys[0]);

    // An invalid part is refused before anything is sent.
    let error = managed
        .submit(
            Request::post("/items")
                .expect("path")
                .idempotency_key("has space"),
        )
        .await
        .expect_err("not visible ASCII");
    assert_unit_error(&error, &ErrorKind::Permanent, SentState::NotSent);
    assert_eq!(server.seen().len(), 2);
}

/// The `idempotency-key` header value of a raw request.
fn idempotency_header(request: &str) -> &str {
    request
        .lines()
        .find_map(|line| line.strip_prefix("idempotency-key: "))
        .expect("an idempotency-key header")
        .trim()
}

// ── credentials ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_rotation_reaches_the_next_unit_over_the_same_transport_and_connection() {
    let server = Server::start(vec![ok("{}"), ok("{}")]).await;
    let harness = Harness::new(Auth::Bearer, HttpConfig::new(&server.base), None);
    harness.api.token.store(token("tok-v1"));
    let managed = harness.managed();

    managed
        .submit(Request::get("/me").expect("path"))
        .await
        .expect("first");
    harness.api.token.store(token("tok-v2"));
    managed
        .submit(Request::get("/me").expect("path"))
        .await
        .expect("second");

    let seen = server.seen();
    assert!(seen[0].contains("authorization: Bearer tok-v1"));
    assert!(seen[1].contains("authorization: Bearer tok-v2"));
    assert_eq!(
        harness.api.creates.load(Ordering::SeqCst),
        1,
        "one transport"
    );
    assert_eq!(server.accepts(), 1, "one kept-alive connection");
}

#[tokio::test]
async fn an_unbound_slot_sends_nothing() {
    let server = Server::start(vec![ok("{}")]).await;
    let harness = Harness::new(Auth::Bearer, HttpConfig::new(&server.base), None);
    let error = harness
        .managed()
        .submit(Request::get("/me").expect("path"))
        .await
        .expect_err("no token");
    assert_unit_error(
        &error,
        &ErrorKind::CredentialUnavailable {
            reason: CredentialUnavailableReason::Absent,
        },
        SentState::NotSent,
    );
    assert_eq!(server.accepts(), 0);
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn basic_and_api_key_headers_have_their_shapes() {
    let server = Server::start(vec![ok("{}"), ok("{}"), ok("{}")]).await;

    let basic = Harness::new(Auth::Basic, HttpConfig::new(&server.base), None);
    basic
        .api
        .login
        .store(Arc::new(CredentialGuard::new(IdentityPassword::new(
            "alice",
            SecretString::new("open sesame"),
        ))));
    basic
        .managed()
        .submit(Request::get("/me").expect("path"))
        .await
        .expect("basic");

    let keyed = Harness::new(Auth::ApiKey, HttpConfig::new(&server.base), None);
    keyed.api.token.store(token("key-7"));
    keyed
        .managed()
        .submit(Request::get("/me").expect("path"))
        .await
        .expect("api key");

    let anonymous = Harness::new(Auth::Anonymous, HttpConfig::new(&server.base), None);
    anonymous
        .managed()
        .submit(Request::get("/me").expect("path"))
        .await
        .expect("anonymous");

    let seen = server.seen();
    assert!(seen[0].contains("authorization: Basic YWxpY2U6b3BlbiBzZXNhbWU="));
    assert!(seen[1].contains("x-api-key: key-7"));
    assert!(!seen[1].contains("authorization"));
    assert!(!seen[2].contains("authorization") && !seen[2].contains("x-api-key"));
}

#[test]
fn a_request_cannot_carry_its_own_credentials() {
    for name in ["Authorization", "proxy-authorization", "Cookie"] {
        let error = Request::get("/me")
            .expect("path")
            .header(name, TOKEN)
            .expect_err("refused before any attempt");
        assert_eq!(*error.kind(), ErrorKind::Permanent);
        assert!(!error.to_string().contains(TOKEN));
    }
}

// ── streamed bodies ──────────────────────────────────────────────────────

fn piece(bytes: &[u8]) -> (Vec<u8>, Duration) {
    (bytes.to_vec(), Duration::ZERO)
}

fn stream_head(length: usize) -> String {
    format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
}

async fn body_of(stream: &mut nebula_sdk::integration::resource::http::ResponseStream) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.expect("chunk"));
    }
    body
}

#[tokio::test]
async fn a_streamed_body_arrives_in_order_then_ends() {
    let server = Server::start(vec![Reply::Chunks(vec![
        piece(stream_head(15).as_bytes()),
        piece(b"alpha"),
        piece(b"beta-"),
        piece(b"gamma"),
    ])])
    .await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let mut stream = open_stream(&managed, Request::get("/feed").expect("path"))
        .await
        .expect("head");
    assert_eq!(stream.status().as_u16(), 200);
    assert_eq!(body_of(&mut stream).await, b"alphabeta-gamma");
    assert!(stream.next().await.is_none(), "ended");
    assert!(server.seen()[0].contains(&format!("authorization: Bearer {TOKEN}")));
}

#[tokio::test]
async fn a_slow_reader_holds_the_server_back() {
    const PIECE: usize = 64 * 1024;
    const PIECES: usize = 512;
    let mut pieces = vec![piece(stream_head(PIECE * PIECES).as_bytes())];
    pieces.extend((0..PIECES).map(|_| piece(&[b'x'; PIECE])));
    let server = Server::start(vec![Reply::Chunks(pieces)]).await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let mut stream = open_stream(&managed, Request::get("/export").expect("path"))
        .await
        .expect("head");
    let first = stream.next().await.expect("a chunk").expect("chunk");

    // A bounded chance for the server to finish if nothing pushed back.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(server.chunks_done(), 0, "the server is held back");
    assert!(server.written() < PIECE * PIECES);

    let rest = body_of(&mut stream).await;
    assert_eq!(first.len() + rest.len(), PIECE * PIECES);
}

/// A head with the first body piece, then a held connection.
fn head_then_hang() -> Reply {
    Reply::HeadersThenHang("HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nfirst".to_owned())
}

#[tokio::test]
async fn a_dropped_stream_disconnects_and_releases_the_lease() {
    let server = Server::start(vec![head_then_hang()]).await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let mut stream = open_stream(&managed, Request::get("/feed").expect("path"))
        .await
        .expect("head");
    assert_eq!(
        stream.next().await.map(Result::ok),
        Some(Some("first".into()))
    );

    drop(stream);
    server.disconnected().await;
    drop(managed);
    tokio::time::timeout(
        Duration::from_secs(10),
        harness.manager.graceful_shutdown(ShutdownConfig::default()),
    )
    .await
    .expect("the drain does not wait on the dropped stream")
    .expect("drained");
}

#[tokio::test]
async fn a_cancelled_stream_settles_cancelled_and_sent() {
    let server = Server::start(vec![head_then_hang()]).await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let mut stream = open_stream(&managed, Request::get("/feed").expect("path"))
        .await
        .expect("head");
    stream.cancel();
    let error = loop {
        match stream.next().await.expect("the unit's error") {
            Ok(_) => {},
            Err(error) => break error,
        }
    };
    assert_unit_error(&error, &ErrorKind::Cancelled, SentState::Sent);
    server.disconnected().await;
}

#[tokio::test]
async fn a_stream_never_polled_or_refused_before_its_grant_sends_nothing() {
    let server = Server::start(vec![ok("{}")]).await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    drop(open_stream(&managed, Request::get("/feed").expect("path")));

    harness.manager.remove(&Api::key()).expect("remove");
    let error = open_stream(&managed, Request::get("/feed").expect("path"))
        .await
        .expect_err("the lease is closing");
    assert_unit_error(&error, &ErrorKind::Cancelled, SentState::NotSent);
    assert_eq!(server.accepts(), 0);
}

#[tokio::test]
async fn the_deadline_mid_body_is_maybe_sent() {
    let server = Server::start(vec![head_then_hang()]).await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let mut stream = open_stream_until(
        &managed,
        Request::post("/export").expect("path").body("{}"),
        Instant::now() + Duration::from_millis(500),
    )
    .await
    .expect("head");
    let error = loop {
        match stream.next().await.expect("the unit's error") {
            Ok(_) => {},
            Err(error) => break error,
        }
    };
    assert_unit_error(&error, &ErrorKind::Transient, SentState::MaybeSent);
    assert!(!error.is_retryable(), "a write with an unknown outcome");
}

#[tokio::test]
async fn removing_the_row_mid_body_cancels_the_stream() {
    let server = Server::start(vec![head_then_hang()]).await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let mut stream = open_stream(&managed, Request::get("/feed").expect("path"))
        .await
        .expect("head");
    harness.manager.remove(&Api::key()).expect("remove");
    let error = loop {
        match stream.next().await.expect("the unit's error") {
            Ok(_) => {},
            Err(error) => break error,
        }
    };
    assert_unit_error(&error, &ErrorKind::Cancelled, SentState::Sent);
}

#[tokio::test]
async fn a_body_cut_short_is_transient_and_sent() {
    let server = Server::start(vec![Reply::HeadersThenDisconnect(
        "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial".to_owned(),
    )])
    .await;
    let harness = Harness::bearer(&server);
    let managed = harness.managed();
    let mut stream = open_stream(&managed, Request::get("/feed").expect("path"))
        .await
        .expect("head");
    let error = loop {
        match stream.next().await.expect("the unit's error") {
            Ok(_) => {},
            Err(error) => break error,
        }
    };
    assert_unit_error(&error, &ErrorKind::Transient, SentState::Sent);
    assert!(error.is_retryable(), "a read");
}

#[tokio::test]
async fn a_body_over_its_budget_is_permanent_and_errors_are_not_streamed() {
    let chunked =
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n0123456789abcdef\r\n0\r\n\r\n";
    let server = Server::start(vec![
        Reply::Bytes(format!("{}{}", stream_head(100), "x".repeat(100))),
        Reply::Bytes(chunked.to_owned()),
        status(404, ""),
    ])
    .await;
    let harness = Harness::new(
        Auth::Bearer,
        HttpConfig::new(&server.base).with_max_stream_bytes(8),
        None,
    );
    harness.api.token.store(token(TOKEN));

    let managed = harness.managed();
    let error = open_stream(&managed, Request::get("/big").expect("path"))
        .await
        .expect_err("declared length over the budget");
    assert_unit_error(&error, &ErrorKind::Permanent, SentState::Sent);

    let managed = harness.managed();
    let mut stream = open_stream(&managed, Request::get("/big").expect("path"))
        .await
        .expect("chunked: no declared length");
    let error = loop {
        match stream.next().await.expect("the unit's error") {
            Ok(_) => {},
            Err(error) => break error,
        }
    };
    assert_unit_error(&error, &ErrorKind::Permanent, SentState::Sent);

    let managed = harness.managed();
    let error = open_stream(&managed, Request::get("/missing").expect("path"))
        .await
        .expect_err("an error status is not streamed");
    assert_unit_error(&error, &ErrorKind::Permanent, SentState::Sent);
    assert_eq!(error.detail(), "Not Found");
}

// ── configuration ────────────────────────────────────────────────────────

#[tokio::test]
async fn an_unusable_configuration_is_refused_at_registration_without_its_url() {
    let base = "https://api.example.com/v1";
    let pem = HttpConfig::new(base).with_extra_root_certificate_pem("not a certificate");
    for config in [
        HttpConfig::new("http://api.example.com/v1"),
        HttpConfig::new("https://user:hunter2@api.example.com/v1"),
        HttpConfig::new("https://api.example.com/v1?key=hunter2"),
        HttpConfig::new(base).with_connect_timeout(Duration::ZERO),
        HttpConfig::new(base).with_request_timeout(Duration::ZERO),
        pem,
    ] {
        let error = config.validate().expect_err("invalid");
        assert_eq!(*error.kind(), ErrorKind::Permanent);
        let refused = Manager::new()
            .register(RegistrationSpec {
                resource: Api {
                    auth: Auth::Anonymous,
                    token: Arc::new(SlotCell::empty()),
                    login: Arc::new(SlotCell::empty()),
                    creates: Arc::default(),
                },
                config,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect_err("registration validates the config");
        for text in [error.to_string(), refused.to_string()] {
            assert!(
                !text.contains("example.com") && !text.contains("hunter2"),
                "{text}"
            );
        }
    }
}
