//! Hermetic behavior tests for the fixed Plane-A OAuth egress policy.
//!
//! Every network test uses a runtime-generated CA and a rustls server bound
//! to loopback. The production resolver still validates a public control
//! answer before the private, `cfg(test)`-only connector override is applied.

use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use secrecy::SecretString;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::{Notify, Semaphore},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

use super::*;

const TEST_HOST: &str = "oauth.test";
const PUBLIC_DNS_CONTROL: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
const CHILD_PROXY_TEST: &str =
    "transport::oauth::egress::behavior_tests::proxy_environment_child_uses_direct_connection";
const CHILD_PROXY_MARKER: &str = "NEBULA_OAUTH_PROXY_CHILD";

#[derive(Clone)]
enum ServerBehavior {
    Response {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    AbortAfterRequest,
    OversizedChunked,
    GateFirstBody {
        body_started: Arc<Notify>,
        release_body: Arc<Notify>,
    },
}

impl ServerBehavior {
    fn json(body: impl Into<Vec<u8>>) -> Self {
        Self::Response {
            status: 200,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.into(),
        }
    }

    fn redirect(status: u16, location: String) -> Self {
        Self::Response {
            status,
            headers: vec![("Location".to_owned(), location)],
            body: Vec::new(),
        }
    }
}

struct ObservedRequests {
    connections: AtomicUsize,
    requests: AtomicUsize,
    posts: AtomicUsize,
    events: Semaphore,
    raw_requests: Mutex<Vec<Vec<u8>>>,
}

impl ObservedRequests {
    fn new() -> Self {
        Self {
            connections: AtomicUsize::new(0),
            requests: AtomicUsize::new(0),
            posts: AtomicUsize::new(0),
            events: Semaphore::new(0),
            raw_requests: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, request: &[u8]) -> usize {
        if request.starts_with(b"POST ") {
            self.posts.fetch_add(1, Ordering::SeqCst);
        }
        let request_number = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
        self.raw_requests
            .lock()
            .expect("raw request observer lock")
            .push(request.to_vec());
        self.events.add_permits(1);
        request_number
    }

    fn raw_requests(&self) -> Vec<Vec<u8>> {
        self.raw_requests
            .lock()
            .expect("raw request observer lock")
            .clone()
    }

    async fn next_event(&self) {
        let permit = timeout(Duration::from_secs(2), self.events.acquire())
            .await
            .expect("request event arrived before timeout")
            .expect("request event semaphore remains open");
        permit.forget();
    }
}

struct TlsFixture {
    addr: SocketAddr,
    trust_anchor: reqwest::Certificate,
    observed: Arc<ObservedRequests>,
    task: JoinHandle<()>,
}

impl TlsFixture {
    async fn spawn(certificate_san: &str, behavior: ServerBehavior) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind TLS fixture");
        let addr = listener.local_addr().expect("TLS fixture local address");
        let (server_config, trust_anchor) = generated_tls(certificate_san);
        let acceptor = TlsAcceptor::from(server_config);
        let observed = Arc::new(ObservedRequests::new());
        let task_observed = Arc::clone(&observed);

        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            while let Ok((stream, _peer)) = listener.accept().await {
                task_observed.connections.fetch_add(1, Ordering::SeqCst);
                let connection_acceptor = acceptor.clone();
                let connection_behavior = behavior.clone();
                let connection_observed = Arc::clone(&task_observed);
                connections.spawn(serve_fixture_connection(
                    stream,
                    connection_acceptor,
                    connection_behavior,
                    connection_observed,
                ));
            }
        });

        Self {
            addr,
            trust_anchor,
            observed,
            task,
        }
    }

    fn endpoint(&self) -> ServerFetchedUrl {
        ServerFetchedUrl::parse(&format!("https://{TEST_HOST}:{}/oauth", self.addr.port()))
            .expect("fixture endpoint satisfies production URL policy")
    }

    fn egress(&self, permits: usize) -> OAuthEgress {
        let resolver = GuardedResolver::for_test(
            Arc::new(StaticPublicLookup),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        );
        let client = build_test_client(resolver, self.trust_anchor.clone())
            .expect("fixed-policy client accepts ephemeral trust anchor");
        OAuthEgress {
            client,
            permits: Arc::new(Semaphore::new(permits)),
        }
    }
}

async fn serve_fixture_connection(
    stream: tokio::net::TcpStream,
    acceptor: TlsAcceptor,
    behavior: ServerBehavior,
    observed: Arc<ObservedRequests>,
) {
    let Ok(mut stream) = acceptor.accept(stream).await else {
        return;
    };
    let Ok(request) = read_request(&mut stream).await else {
        return;
    };
    let request_number = observed.record(&request);
    let _ = respond(&mut stream, &behavior, request_number).await;
}

impl Drop for TlsFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct CountingTcpFixture {
    addr: SocketAddr,
    connections: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl CountingTcpFixture {
    async fn spawn() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind target fixture");
        let addr = listener.local_addr().expect("target fixture local address");
        let connections = Arc::new(AtomicUsize::new(0));
        let task_connections = Arc::clone(&connections);
        let task = tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                task_connections.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });
        Self {
            addr,
            connections,
            task,
        }
    }
}

impl Drop for CountingTcpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct StaticPublicLookup;

impl HostLookup for StaticPublicLookup {
    fn lookup(&self, _host: String) -> LookupFuture {
        Box::pin(std::future::ready(Ok(vec![PUBLIC_DNS_CONTROL])))
    }
}

fn generated_tls(certificate_san: &str) -> (Arc<ServerConfig>, reqwest::Certificate) {
    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .expect("an empty CA subject-alt-name set is valid");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_key = KeyPair::generate().expect("generate ephemeral CA key");
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .expect("self-sign ephemeral CA");
    let issuer = Issuer::new(ca_params, ca_key);

    let mut leaf_params = CertificateParams::new(vec![certificate_san.to_owned()])
        .expect("fixture SAN is a valid DNS name");
    leaf_params.use_authority_key_identifier_extension = true;
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate().expect("generate ephemeral leaf key");
    let leaf_cert = leaf_params
        .signed_by(&leaf_key, &issuer)
        .expect("sign ephemeral leaf certificate");
    let private_key = PrivatePkcs8KeyDer::from(leaf_key.serialize_der());
    let mut server_config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("AWS-LC supports the fixture TLS protocol versions")
    .with_no_client_auth()
    .with_single_cert(vec![leaf_cert.der().clone()], private_key.into())
    .expect("ephemeral certificate and key match");
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    let trust_anchor =
        reqwest::Certificate::from_der(ca_cert.der().as_ref()).expect("ephemeral CA DER is valid");
    (Arc::new(server_config), trust_anchor)
}

async fn read_request<S>(stream: &mut S) -> io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    const MAX_REQUEST_BYTES: usize = 64 * 1024;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    let mut expected_len = None;

    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before its declared body",
            ));
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture request exceeded cap",
            ));
        }

        if expected_len.is_none()
            && let Some(header_end) = find_bytes(&request, b"\r\n\r\n")
        {
            let body_start = header_end + 4;
            let headers = std::str::from_utf8(&request[..header_end]).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "request headers were not UTF-8")
            })?;
            let body_len = headers
                .split("\r\n")
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _value)| name.eq_ignore_ascii_case("content-length"))
                .map_or(Ok(0_usize), |(_name, value)| {
                    value.trim().parse::<usize>().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "invalid content length")
                    })
                })?;
            expected_len = Some(body_start.saturating_add(body_len));
        }

        if expected_len.is_some_and(|expected| request.len() >= expected) {
            return Ok(request);
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn respond<S>(
    stream: &mut S,
    behavior: &ServerBehavior,
    request_number: usize,
) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    match behavior {
        ServerBehavior::Response {
            status,
            headers,
            body,
        } => write_response(stream, *status, headers, body).await,
        ServerBehavior::AbortAfterRequest => Ok(()),
        ServerBehavior::OversizedChunked => write_oversized_chunked(stream).await,
        ServerBehavior::GateFirstBody {
            body_started,
            release_body,
        } if request_number == 1 => {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nx")
                .await?;
            stream.flush().await?;
            body_started.notify_one();
            release_body.notified().await;
            stream.write_all(b"y").await
        },
        ServerBehavior::GateFirstBody { .. } => write_response(stream, 200, &[], b"ok").await,
    }
}

async fn write_response<S>(
    stream: &mut S,
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let reason = match status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        _ => "Fixture Response",
    };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await
}

async fn write_oversized_chunked<S>(stream: &mut S) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
        .await?;
    let chunk = vec![b'x'; 64 * 1024];
    for _ in 0..4 {
        if write_chunk(stream, &chunk).await.is_err() {
            return Ok(());
        }
    }
    if write_chunk(stream, b"x").await.is_err() {
        return Ok(());
    }
    let _ = stream.write_all(b"0\r\n\r\n").await;
    Ok(())
}

async fn write_chunk<S>(stream: &mut S, chunk: &[u8]) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(format!("{:X}\r\n", chunk.len()).as_bytes())
        .await?;
    stream.write_all(chunk).await?;
    stream.write_all(b"\r\n").await
}

#[tokio::test]
async fn matching_tls_san_succeeds_with_the_fixed_client_policy() {
    let fixture = TlsFixture::spawn(TEST_HOST, ServerBehavior::json(b"{}".to_vec())).await;
    let result = fixture
        .egress(1)
        .fetch_discovery(&fixture.endpoint())
        .await
        .expect("trusted CA and matching SAN must succeed");

    assert_eq!(result.as_slice(), b"{}");
    assert_eq!(fixture.observed.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mismatched_tls_san_fails_before_any_http_request() {
    let fixture = TlsFixture::spawn("wrong.test", ServerBehavior::json(b"{}".to_vec())).await;
    let result = fixture.egress(1).fetch_discovery(&fixture.endpoint()).await;

    assert_eq!(
        result.expect_err("SAN mismatch must fail"),
        OAuthFailureCode::DiscoveryUnavailable
    );
    assert_eq!(fixture.observed.requests.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn every_redirect_status_refuses_https_follow_and_http_downgrade() {
    for status in [301, 302, 303, 307, 308] {
        for scheme in ["https", "http"] {
            let target = CountingTcpFixture::spawn().await;
            let location = format!("{scheme}://target.test:{}/capture", target.addr.port());
            let source =
                TlsFixture::spawn(TEST_HOST, ServerBehavior::redirect(status, location)).await;
            let result = source.egress(1).fetch_discovery(&source.endpoint()).await;

            assert_eq!(
                result.expect_err("redirect response is not a successful provider response"),
                OAuthFailureCode::DiscoveryUnavailable,
                "status {status} with {scheme} target"
            );
            sleep(Duration::from_millis(25)).await;
            assert_eq!(
                target.connections.load(Ordering::SeqCst),
                0,
                "status {status} followed a {scheme} redirect"
            );
            assert_eq!(source.observed.requests.load(Ordering::SeqCst), 1);
        }
    }

    assert!(
        ServerFetchedUrl::parse("http://provider.example/token").is_err(),
        "a direct server-fetched HTTP endpoint must also fail closed"
    );
}

#[tokio::test]
async fn aborted_token_exchange_is_never_retried() {
    let fixture = TlsFixture::spawn(TEST_HOST, ServerBehavior::AbortAfterRequest).await;
    let client_id = SecretString::new("client".into());
    let client_secret = SecretString::new("secret".into());
    let result = fixture
        .egress(1)
        .exchange_token(TokenExchangeRequest {
            endpoint: &fixture.endpoint(),
            auth_method: TokenEndpointAuthMethod::ClientSecretPost,
            client_id: &client_id,
            client_secret: &client_secret,
            code: "authorization-code",
            redirect_uri: "https://nebula.example/callback",
            code_verifier: "pkce-verifier",
        })
        .await;

    assert_eq!(
        result.expect_err("connection abort must fail token exchange"),
        OAuthFailureCode::TokenExchangeFailed
    );
    sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.observed.posts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.requests.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn token_endpoint_authentication_uses_exactly_one_configured_method() {
    for (client_id, client_secret, expected_decoded) in [
        ("client:id", "secret:colon", "client%3Aid:secret%3Acolon"),
        (
            "id%+ space café",
            "sëcret +%",
            "id%25%2B+space+caf%C3%A9:s%C3%ABcret+%2B%25",
        ),
    ] {
        let basic_fixture =
            TlsFixture::spawn(TEST_HOST, ServerBehavior::json(b"{}".to_vec())).await;
        let client_id = SecretString::new(client_id.into());
        let client_secret = SecretString::new(client_secret.into());
        basic_fixture
            .egress(1)
            .exchange_token(TokenExchangeRequest {
                endpoint: &basic_fixture.endpoint(),
                auth_method: TokenEndpointAuthMethod::ClientSecretBasic,
                client_id: &client_id,
                client_secret: &client_secret,
                code: "code",
                redirect_uri: "https://nebula.example/callback",
                code_verifier: "verifier",
            })
            .await
            .expect("Basic token request must reach fixture");
        let basic_raw = basic_fixture.observed.raw_requests();
        let basic_raw = String::from_utf8_lossy(&basic_raw[0]);
        let (basic_headers, basic_body) = basic_raw
            .split_once("\r\n\r\n")
            .expect("request has headers and form body");
        let basic_value = basic_headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.trim())
            .and_then(|value| value.strip_prefix("Basic "))
            .expect("token request has a Basic authorization value");
        let decoded = STANDARD
            .decode(basic_value)
            .expect("Basic credential is valid base64");
        assert_eq!(
            String::from_utf8(decoded).expect("encoded client credentials remain ASCII"),
            expected_decoded,
            "each credential component is form-urlencoded before the Basic colon"
        );
        assert!(!basic_body.contains("client_id="));
        assert!(!basic_body.contains("client_secret="));
    }

    let post_fixture = TlsFixture::spawn(TEST_HOST, ServerBehavior::json(b"{}".to_vec())).await;
    post_fixture
        .egress(1)
        .exchange_token(TokenExchangeRequest {
            endpoint: &post_fixture.endpoint(),
            auth_method: TokenEndpointAuthMethod::ClientSecretPost,
            client_id: &SecretString::new("post-client-canary".into()),
            client_secret: &SecretString::new("post-secret-canary".into()),
            code: "code",
            redirect_uri: "https://nebula.example/callback",
            code_verifier: "verifier",
        })
        .await
        .expect("form-auth token request must reach fixture");
    let post_raw = post_fixture.observed.raw_requests();
    let post_raw = String::from_utf8_lossy(&post_raw[0]);
    let (post_headers, post_body) = post_raw
        .split_once("\r\n\r\n")
        .expect("request has headers and form body");
    assert!(!post_headers.to_ascii_lowercase().contains("authorization:"));
    assert!(post_body.contains("client_id=post-client-canary"));
    assert!(post_body.contains("client_secret=post-secret-canary"));
}

#[tokio::test]
async fn chunked_body_over_the_cap_fails_closed() {
    let fixture = TlsFixture::spawn(TEST_HOST, ServerBehavior::OversizedChunked).await;
    let result = fixture.egress(1).fetch_discovery(&fixture.endpoint()).await;

    assert_eq!(
        result.expect_err("chunked body above the cap must fail"),
        OAuthFailureCode::DiscoveryUnavailable
    );
    assert_eq!(fixture.observed.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn concurrency_permit_is_held_until_the_response_body_finishes() {
    let body_started = Arc::new(Notify::new());
    let release_body = Arc::new(Notify::new());
    let fixture = TlsFixture::spawn(
        TEST_HOST,
        ServerBehavior::GateFirstBody {
            body_started: Arc::clone(&body_started),
            release_body: Arc::clone(&release_body),
        },
    )
    .await;
    let egress = Arc::new(fixture.egress(1));
    let endpoint = fixture.endpoint();

    let first_egress = Arc::clone(&egress);
    let first_endpoint = endpoint.clone();
    let first = tokio::spawn(async move { first_egress.fetch_discovery(&first_endpoint).await });
    fixture.observed.next_event().await;
    timeout(Duration::from_secs(2), body_started.notified())
        .await
        .expect("first response body reached its gate");

    let second_egress = Arc::clone(&egress);
    let second_endpoint = endpoint.clone();
    let second = tokio::spawn(async move { second_egress.fetch_discovery(&second_endpoint).await });
    assert!(
        timeout(
            Duration::from_millis(150),
            fixture.observed.events.acquire()
        )
        .await
        .is_err(),
        "second request escaped while the first response retained the only permit"
    );

    release_body.notify_one();
    let first_body = first
        .await
        .expect("first fetch task joined")
        .expect("first response completed");
    assert_eq!(first_body.as_slice(), b"xy");

    fixture.observed.next_event().await;
    let second_body = second
        .await
        .expect("second fetch task joined")
        .expect("second response completed");
    assert_eq!(second_body.as_slice(), b"ok");
    assert_eq!(fixture.observed.requests.load(Ordering::SeqCst), 2);
}

#[test]
fn proxy_environment_is_ignored_in_an_isolated_subprocess() {
    let proxy_listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("bind isolated proxy observer");
    proxy_listener
        .set_nonblocking(true)
        .expect("set proxy observer nonblocking");
    let proxy_addr = proxy_listener.local_addr().expect("proxy observer address");
    let proxy_connections = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_connections = Arc::clone(&proxy_connections);
    let thread_stop = Arc::clone(&stop);
    let observer = thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            match proxy_listener.accept() {
                Ok((stream, _peer)) => {
                    thread_connections.fetch_add(1, Ordering::SeqCst);
                    drop(stream);
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                },
                Err(_) => break,
            }
        }
    });

    let proxy_url = format!("http://{proxy_addr}");
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--ignored", "--exact", CHILD_PROXY_TEST, "--nocapture"])
        .env(CHILD_PROXY_MARKER, "1")
        .env("HTTPS_PROXY", &proxy_url)
        .env("https_proxy", &proxy_url)
        .env("HTTP_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env("all_proxy", &proxy_url)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .expect("run isolated proxy-policy child test");

    stop.store(true, Ordering::SeqCst);
    observer.join().expect("proxy observer thread joined");
    assert!(
        output.status.success(),
        "proxy-policy child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        proxy_connections.load(Ordering::SeqCst),
        0,
        "fixed client consulted a process proxy"
    );
}

#[test]
#[ignore = "spawned by proxy_environment_is_ignored_in_an_isolated_subprocess"]
fn proxy_environment_child_uses_direct_connection() {
    if std::env::var_os(CHILD_PROXY_MARKER).is_none() {
        return;
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build child test runtime");
    runtime.block_on(async {
        let fixture = TlsFixture::spawn(TEST_HOST, ServerBehavior::json(b"{}".to_vec())).await;
        let body = fixture
            .egress(1)
            .fetch_discovery(&fixture.endpoint())
            .await
            .expect("no-proxy client connects directly to TLS fixture");
        assert_eq!(body.as_slice(), b"{}");
        assert_eq!(fixture.observed.requests.load(Ordering::SeqCst), 1);
    });
}
