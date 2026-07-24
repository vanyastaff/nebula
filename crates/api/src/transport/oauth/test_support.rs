//! Private real-TLS fixture for OAuth runtime semantic tests.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObservedRequest {
    pub(crate) method: String,
    pub(crate) path: String,
}

#[derive(Clone)]
pub(crate) struct TestResponse {
    status: u16,
    body: Vec<u8>,
    delay: Duration,
}

impl TestResponse {
    pub(crate) fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn failure(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

type Responder = dyn Fn(&ObservedRequest, usize) -> TestResponse + Send + Sync;

pub(crate) struct TlsFixture {
    addr: SocketAddr,
    trust_anchor: reqwest::Certificate,
    requests: Arc<Mutex<Vec<ObservedRequest>>>,
    task: JoinHandle<()>,
}

impl TlsFixture {
    pub(crate) async fn spawn(
        certificate_san: &str,
        responder: impl Fn(&ObservedRequest, usize) -> TestResponse + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind OAuth TLS fixture");
        let addr = listener
            .local_addr()
            .expect("read OAuth TLS fixture address");
        let (server_config, trust_anchor) = generated_tls(certificate_san);
        let acceptor = TlsAcceptor::from(server_config);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let task_requests = Arc::clone(&requests);
        let responder: Arc<Responder> = Arc::new(responder);

        let task = tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let requests = Arc::clone(&task_requests);
                let responder = Arc::clone(&responder);
                tokio::spawn(serve_connection(stream, acceptor, requests, responder));
            }
        });

        Self {
            addr,
            trust_anchor,
            requests,
            task,
        }
    }

    pub(crate) fn endpoint(&self, path: &str) -> String {
        format!("https://oauth.test:{}{path}", self.addr.port())
    }

    pub(crate) fn trust_anchor(&self) -> reqwest::Certificate {
        self.trust_anchor.clone()
    }

    pub(crate) fn requests(&self) -> Vec<ObservedRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) async fn wait_for_request_count(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if self.requests().len() >= expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("OAuth TLS fixture observed expected request count");
    }
}

async fn serve_connection(
    stream: tokio::net::TcpStream,
    acceptor: TlsAcceptor,
    requests: Arc<Mutex<Vec<ObservedRequest>>>,
    responder: Arc<Responder>,
) {
    let Ok(mut stream) = acceptor.accept(stream).await else {
        return;
    };
    let Ok(raw) = read_request(&mut stream).await else {
        return;
    };
    let Some(request) = parse_request_line(&raw) else {
        return;
    };
    let request_number = {
        let mut observed = requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        observed.push(request.clone());
        observed.len()
    };
    let response = responder(&request, request_number);
    if !response.delay.is_zero() {
        tokio::time::sleep(response.delay).await;
    }
    let _ = write_response(&mut stream, &response).await;
}

impl Drop for TlsFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn generated_tls(certificate_san: &str) -> (Arc<ServerConfig>, reqwest::Certificate) {
    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .expect("empty CA subject-alt-name set is valid");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_key = KeyPair::generate().expect("generate ephemeral CA key");
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .expect("self-sign ephemeral CA");
    let issuer = Issuer::new(ca_params, ca_key);

    let mut leaf_params = CertificateParams::new(vec![certificate_san.to_owned()])
        .expect("fixture SAN must be a valid DNS name");
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
    .expect("ephemeral certificate and key must match");
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    let trust_anchor = reqwest::Certificate::from_der(ca_cert.der().as_ref())
        .expect("ephemeral CA DER must be valid");
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
                "request exceeded fixture cap",
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
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map_or(Ok(0_usize), |(_, value)| {
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

fn parse_request_line(raw: &[u8]) -> Option<ObservedRequest> {
    let line_end = find_bytes(raw, b"\r\n")?;
    let line = std::str::from_utf8(&raw[..line_end]).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let path = target.split('?').next().unwrap_or(target).to_owned();
    Some(ObservedRequest { method, path })
}

async fn write_response<S>(stream: &mut S, response: &TestResponse) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Fixture Response",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await
}
