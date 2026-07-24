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

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use nebula_core::CredentialId;
use nebula_credential::{
    AuthStyle, Credential, CredentialContext, CredentialHandle, CredentialState, OAuth2Credential,
    OAuth2State, OAuth2Token, RefreshNotAppliedPhase, SecretString,
    runtime::refresh::{
        OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES, RefreshCoordConfig, RefreshCoordinator,
        RefreshTransport,
    },
    runtime::{CredentialResolver, ResolveError},
    serde_secret,
};
use nebula_storage::credential::SqliteCredentialPersistence;
use nebula_storage_port::{
    CredentialCreate, CredentialOwner, CredentialPersistence, CredentialSelector,
    store::{RefreshClaimStore, ReplicaId},
};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
    time::sleep,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

use super::ReqwestRefreshTransport;

const TEST_HOST: &str = "oauth-refresh.test";
const PUBLIC_DNS_CONTROL: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
const CHILD_PROXY_TEST: &str = "credential_adapters::transport_security_tests::\
proxy_environment_child_uses_direct_connection";
const CHILD_PROXY_MARKER: &str = "NEBULA_CREDENTIAL_REFRESH_PROXY_CHILD";

#[derive(Clone, Copy)]
enum ServerBehavior {
    Success,
    Redirect(u16),
    AbortAfterRequest,
    OversizedContentLength,
    OversizedChunkedBody,
    ExactBodyLimit,
}

struct TlsFixture {
    addr: SocketAddr,
    trust_anchor: reqwest::Certificate,
    connections: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    request_bytes: Arc<Mutex<Vec<Vec<u8>>>>,
    task: JoinHandle<()>,
}

impl TlsFixture {
    async fn spawn(behavior: ServerBehavior) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind TLS fixture");
        let addr = listener.local_addr().expect("TLS fixture address");
        let (server_config, trust_anchor) = generated_tls(TEST_HOST);
        let acceptor = TlsAcceptor::from(server_config);
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let request_bytes = Arc::new(Mutex::new(Vec::new()));
        let task_connections = Arc::clone(&connections);
        let task_requests = Arc::clone(&requests);
        let task_request_bytes = Arc::clone(&request_bytes);
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            while let Ok((stream, _peer)) = listener.accept().await {
                task_connections.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let requests = Arc::clone(&task_requests);
                let request_bytes = Arc::clone(&task_request_bytes);
                tasks.spawn(async move {
                    let Ok(mut stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let Ok(request) = read_request(&mut stream).await else {
                        return;
                    };
                    request_bytes
                        .lock()
                        .expect("request capture lock")
                        .push(request);
                    requests.fetch_add(1, Ordering::SeqCst);
                    if matches!(behavior, ServerBehavior::AbortAfterRequest) {
                        return;
                    }
                    let _ = write_response(&mut stream, behavior, addr.port()).await;
                });
            }
        });

        Self {
            addr,
            trust_anchor,
            connections,
            requests,
            request_bytes,
            task,
        }
    }

    fn endpoint(&self) -> String {
        format!("https://{TEST_HOST}:{}/token", self.addr.port())
    }

    fn transport(&self, dns_answers: Vec<IpAddr>) -> ReqwestRefreshTransport {
        ReqwestRefreshTransport::for_test(
            self.trust_anchor.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            dns_answers,
        )
        .expect("fixed test client")
    }

    fn last_request(&self) -> Vec<u8> {
        self.request_bytes
            .lock()
            .expect("request capture lock")
            .last()
            .expect("fixture captured one request")
            .clone()
    }
}

impl Drop for TlsFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn generated_tls(certificate_san: &str) -> (Arc<ServerConfig>, reqwest::Certificate) {
    let mut ca_params =
        CertificateParams::new(Vec::<String>::new()).expect("empty CA SAN set is valid");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_key = KeyPair::generate().expect("generate ephemeral CA key");
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .expect("self-sign ephemeral CA");
    let issuer = Issuer::new(ca_params, ca_key);

    let mut leaf_params =
        CertificateParams::new(vec![certificate_san.to_owned()]).expect("valid fixture SAN");
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
    .expect("fixture crypto provider supports safe protocols")
    .with_no_client_auth()
    .with_single_cert(vec![leaf_cert.der().clone()], private_key.into())
    .expect("ephemeral certificate and key match");
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    let trust_anchor =
        reqwest::Certificate::from_der(ca_cert.der().as_ref()).expect("valid ephemeral CA");
    (Arc::new(server_config), trust_anchor)
}

async fn read_request(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> io::Result<Vec<u8>> {
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
                "request exceeded fixture cap",
            ));
        }

        if expected_len.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let body_start = header_end + 4;
            let headers = std::str::from_utf8(&request[..header_end]).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "request headers are not UTF-8")
            })?;
            let body_len = headers
                .split("\r\n")
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map_or(Ok(0_usize), |(_, value)| {
                    value.trim().parse::<usize>().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length")
                    })
                })?;
            expected_len = Some(body_start.saturating_add(body_len));
        }

        if expected_len.is_some_and(|expected| request.len() >= expected) {
            return Ok(request);
        }
    }
}

async fn write_response(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    behavior: ServerBehavior,
    port: u16,
) -> io::Result<()> {
    const BODY_CANARY: &[u8] = b"response-body-diagnostic-canary";

    match behavior {
        ServerBehavior::AbortAfterRequest => return Ok(()),
        ServerBehavior::OversizedContentLength => {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES.saturating_add(1)
            );
            stream.write_all(response.as_bytes()).await?;
            return stream.write_all(BODY_CANARY).await;
        },
        ServerBehavior::OversizedChunkedBody => {
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await?;
            let mut body = vec![b'x'; OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES.saturating_add(1)];
            body[..BODY_CANARY.len()].copy_from_slice(BODY_CANARY);
            stream
                .write_all(format!("{:X}\r\n", body.len()).as_bytes())
                .await?;
            stream.write_all(&body).await?;
            return stream.write_all(b"\r\n0\r\n\r\n").await;
        },
        ServerBehavior::ExactBodyLimit => {
            const PREFIX: &[u8] = br#"{"error":"invalid_request","padding":""#;
            const SUFFIX: &[u8] = br#""}"#;
            let mut body = Vec::with_capacity(OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES);
            body.extend_from_slice(PREFIX);
            body.extend_from_slice(BODY_CANARY);
            body.resize(
                OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES.saturating_sub(SUFFIX.len()),
                b'x',
            );
            body.extend_from_slice(SUFFIX);
            assert_eq!(body.len(), OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES);
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await?;
            return stream.write_all(&body).await;
        },
        ServerBehavior::Success | ServerBehavior::Redirect(_) => {},
    }

    let (status, extra_header, body) = match behavior {
        ServerBehavior::Success => (
            "200 OK",
            String::new(),
            br#"{"access_token":"new-access","token_type":"Bearer","scope":"read"}"#.as_slice(),
        ),
        ServerBehavior::Redirect(status) => (
            if status == 307 {
                "307 Temporary Redirect"
            } else {
                "308 Permanent Redirect"
            },
            format!("Location: https://{TEST_HOST}:{port}/second\r\n"),
            b"".as_slice(),
        ),
        ServerBehavior::AbortAfterRequest
        | ServerBehavior::OversizedContentLength
        | ServerBehavior::OversizedChunkedBody
        | ServerBehavior::ExactBodyLimit => {
            unreachable!("special response behaviors return before static response assembly")
        },
    };
    let response = format!(
        "HTTP/1.1 {status}\r\n{extra_header}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await
}

fn oauth_state(endpoint: String) -> OAuth2State {
    OAuth2State {
        access_token: SecretString::new("old-access"),
        token_type: "Bearer".to_owned(),
        refresh_token: Some(SecretString::new("refresh-secret")),
        expires_at: Some(
            "2000-01-01T00:00:00Z"
                .parse()
                .expect("fixed RFC 3339 timestamp"),
        ),
        scopes: vec!["read".to_owned()],
        client_id: SecretString::new("client"),
        client_secret: SecretString::new("client-secret"),
        token_url: endpoint,
        auth_style: AuthStyle::PostBody,
    }
}

async fn refresh_through_resolver(
    state: OAuth2State,
    transport: ReqwestRefreshTransport,
) -> Result<CredentialHandle<OAuth2Token>, ResolveError> {
    let expires_at = state.expires_at;
    let data = serde_secret::expose_for_serialization(|| serde_json::to_vec(&state))
        .expect("serialize OAuth2 test state into the trusted persistence fixture");
    let store = Arc::new(
        SqliteCredentialPersistence::connect_memory()
            .await
            .expect("open isolated credential store"),
    );
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("server-transport-test"),
        CredentialId::new(),
    );
    store
        .create(
            &selector,
            CredentialCreate::new(
                OAuth2Credential::KEY.to_owned(),
                data.into(),
                OAuth2State::KIND.to_owned(),
                OAuth2State::VERSION,
                None,
                expires_at,
                false,
                serde_json::Map::new(),
            ),
        )
        .await
        .expect("seed expired OAuth2 credential");

    let claims: Arc<dyn RefreshClaimStore> = Arc::new(store.refresh_claim_repo());
    let coordinator = RefreshCoordinator::new_with(
        claims,
        ReplicaId::new("server-transport-test"),
        RefreshCoordConfig::default(),
    )
    .expect("default refresh coordinator configuration");
    let transport: Arc<dyn RefreshTransport> = Arc::new(transport);
    let resolver = CredentialResolver::with_dependencies(store, Arc::new(coordinator), transport);

    resolver
        .resolve_with_refresh::<OAuth2Credential>(
            &selector,
            &CredentialContext::for_owner("server-transport-test"),
        )
        .await
}

#[tokio::test]
async fn redirect_307_and_308_never_issue_a_second_request() {
    for status in [307, 308] {
        let fixture = TlsFixture::spawn(ServerBehavior::Redirect(status)).await;
        let transport = fixture.transport(vec![PUBLIC_DNS_CONTROL]);
        let state = oauth_state(fixture.endpoint());

        let error = refresh_through_resolver(state, transport)
            .await
            .expect_err("redirect response must fail refresh");

        assert!(matches!(error, ResolveError::ProviderOutcomeUnknown { .. }));
        sleep(Duration::from_millis(50)).await;
        assert_eq!(
            fixture.requests.load(Ordering::SeqCst),
            1,
            "status {status} triggered a second HTTP request"
        );
        assert_eq!(
            fixture.connections.load(Ordering::SeqCst),
            1,
            "status {status} triggered a second connection"
        );
    }
}

#[tokio::test]
async fn private_and_mixed_dns_answers_fail_before_connect() {
    for answers in [
        vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))],
        vec![
            PUBLIC_DNS_CONTROL,
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
        ],
        vec!["fc00::1".parse().expect("valid ULA")],
    ] {
        let fixture = TlsFixture::spawn(ServerBehavior::Success).await;
        let transport = fixture.transport(answers);
        let state = oauth_state(fixture.endpoint());

        let error = refresh_through_resolver(state, transport)
            .await
            .expect_err("non-global DNS set must fail");

        assert!(matches!(error, ResolveError::ProviderOutcomeUnknown { .. }));
        assert_eq!(fixture.connections.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn aborted_provider_post_is_never_retried() {
    let fixture = TlsFixture::spawn(ServerBehavior::AbortAfterRequest).await;
    let transport = fixture.transport(vec![PUBLIC_DNS_CONTROL]);
    let state = oauth_state(fixture.endpoint());

    let error = refresh_through_resolver(state, transport)
        .await
        .expect_err("aborted response must fail refresh");

    assert!(matches!(error, ResolveError::ProviderOutcomeUnknown { .. }));
    sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fixture.requests.load(Ordering::SeqCst),
        1,
        "reqwest retried an acknowledged provider POST"
    );
    assert_eq!(
        fixture.connections.load(Ordering::SeqCst),
        1,
        "reqwest opened a retry connection"
    );
}

#[tokio::test]
async fn header_auth_wire_uses_rfc6749_form_encoded_basic_components() {
    const RAW_CLIENT_ID: &str = "id:%+ snow 雪";
    const RAW_CLIENT_SECRET: &str = "secret:%+ lock 🔒";
    const ENCODED_CLIENT_ID: &str = "id%3A%25%2B+snow+%E9%9B%AA";
    const ENCODED_CLIENT_SECRET: &str = "secret%3A%25%2B+lock+%F0%9F%94%92";

    let fixture = TlsFixture::spawn(ServerBehavior::Success).await;
    let transport = fixture.transport(vec![PUBLIC_DNS_CONTROL]);
    let mut state = oauth_state(fixture.endpoint());
    state.auth_style = AuthStyle::Header;
    state.client_id = SecretString::new(RAW_CLIENT_ID);
    state.client_secret = SecretString::new(RAW_CLIENT_SECRET);

    let handle = refresh_through_resolver(state, transport)
        .await
        .expect("RFC 6749 Basic request receives valid response");
    assert_eq!(
        handle.snapshot().access_token().expose_secret(),
        "new-access"
    );

    let request = fixture.last_request();
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("captured request has a header terminator");
    let headers = std::str::from_utf8(&request[..header_end]).expect("request headers are UTF-8");
    let expected = format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("{ENCODED_CLIENT_ID}:{ENCODED_CLIENT_SECRET}"))
    );
    let authorization = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.trim());

    assert!(
        authorization.is_some_and(|value| value == expected),
        "Authorization header must encode each raw component before the Basic join"
    );
}

#[tokio::test]
async fn oversized_response_bodies_fail_with_closed_diagnostics() {
    for behavior in [
        ServerBehavior::OversizedContentLength,
        ServerBehavior::OversizedChunkedBody,
    ] {
        let fixture = TlsFixture::spawn(behavior).await;
        let transport = fixture.transport(vec![PUBLIC_DNS_CONTROL]);
        let state = oauth_state(fixture.endpoint());

        let error = refresh_through_resolver(state, transport)
            .await
            .expect_err("response above the fixed cap must fail");
        assert!(matches!(error, ResolveError::ProviderOutcomeUnknown { .. }));
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains("response-body-diagnostic-canary"));
        assert!(
            error
                .to_string()
                .ends_with("provider refresh outcome is unknown after dispatch")
        );
    }
}

#[tokio::test]
async fn exact_response_body_limit_crosses_transport_and_is_interpreted() {
    let fixture = TlsFixture::spawn(ServerBehavior::ExactBodyLimit).await;
    let transport = fixture.transport(vec![PUBLIC_DNS_CONTROL]);
    let state = oauth_state(fixture.endpoint());

    let error = refresh_through_resolver(state, transport)
        .await
        .expect_err("provider-confirmed invalid_request must not refresh");

    let ResolveError::RefreshNotApplied { context, .. } = &error else {
        panic!("exact cap response must be completely read and interpreted: {error:?}");
    };
    assert_eq!(
        context.phase(),
        RefreshNotAppliedPhase::ProviderConfirmedNotApplied
    );
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains("response-body-diagnostic-canary"));
}

#[test]
fn proxy_environment_is_ignored_in_an_isolated_subprocess() {
    let proxy_listener =
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind proxy observer");
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
        "fixed refresh client consulted a process proxy"
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
        let fixture = TlsFixture::spawn(ServerBehavior::Success).await;
        let transport = fixture.transport(vec![PUBLIC_DNS_CONTROL]);
        let state = oauth_state(fixture.endpoint());
        let handle = refresh_through_resolver(state, transport)
            .await
            .expect("no-proxy client connects directly");
        assert_eq!(
            handle.snapshot().access_token().expose_secret(),
            "new-access"
        );
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
    });
}
