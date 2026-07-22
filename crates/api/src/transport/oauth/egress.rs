//! Fixed outbound policy for Plane-A OAuth identity traffic.
//!
//! The IP policy is a conservative denylist derived from the IANA IPv4 and
//! IPv6 Special-Purpose Address Registries, last updated 2025-10-09. Literal
//! hosts and DNS answers pass through the same classifier. The resolver
//! returns only the already-validated exact addresses to reqwest.

use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use futures::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Semaphore;
use url::{Host, Url};
use zeroize::Zeroizing;

use super::error::{OAuthFailureCode, OAuthRuntimeBuildError};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_DNS_ANSWERS: usize = 32;
const MAX_OUTBOUND_REQUESTS: usize = 64;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

type LookupFuture = Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, LookupError>> + Send>>;

/// Private token-endpoint client-authentication policy. Production callers
/// cannot select this value: GitHub's fixed profile uses form auth while OIDC
/// discovery admits only the two reviewed secret-based methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TokenEndpointAuthMethod {
    ClientSecretBasic,
    ClientSecretPost,
}

pub(super) struct TokenExchangeRequest<'a> {
    pub(super) endpoint: &'a ServerFetchedUrl,
    pub(super) auth_method: TokenEndpointAuthMethod,
    pub(super) client_id: &'a SecretString,
    pub(super) client_secret: &'a SecretString,
    pub(super) code: &'a str,
    pub(super) redirect_uri: &'a str,
    pub(super) code_verifier: &'a str,
}

#[derive(Debug)]
enum LookupError {
    Failed,
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OAuth DNS lookup failed")
    }
}

impl std::error::Error for LookupError {}

trait HostLookup: Send + Sync {
    fn lookup(&self, host: String) -> LookupFuture;
}

struct SystemLookup;

impl HostLookup for SystemLookup {
    fn lookup(&self, host: String) -> LookupFuture {
        Box::pin(async move {
            tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map(|answers| {
                    answers
                        .take(MAX_DNS_ANSWERS.saturating_add(1))
                        .map(|answer| answer.ip())
                        .collect()
                })
                .map_err(|_| LookupError::Failed)
        })
    }
}

#[cfg(test)]
struct StaticTestLookup {
    answers: Vec<IpAddr>,
}

#[cfg(test)]
impl HostLookup for StaticTestLookup {
    fn lookup(&self, _host: String) -> LookupFuture {
        Box::pin(std::future::ready(Ok(self.answers.clone())))
    }
}

#[derive(Clone)]
struct GuardedResolver {
    lookup: Arc<dyn HostLookup>,
    #[cfg(test)]
    connect_override: Option<IpAddr>,
}

impl GuardedResolver {
    fn system() -> Self {
        Self {
            lookup: Arc::new(SystemLookup),
            #[cfg(test)]
            connect_override: None,
        }
    }

    /// Build the resolver used by hermetic TLS tests.
    ///
    /// The supplied lookup still runs through the production answer policy.
    /// Only after it succeeds is the socket IP replaced with the exact local
    /// fixture address. This seam is private and absent from non-test builds.
    #[cfg(test)]
    fn for_test(lookup: Arc<dyn HostLookup>, connect_override: IpAddr) -> Self {
        Self {
            lookup,
            connect_override: Some(connect_override),
        }
    }

    fn validate_answers(answers: Vec<IpAddr>) -> Result<Vec<SocketAddr>, LookupError> {
        if answers.is_empty()
            || answers.len() > MAX_DNS_ANSWERS
            || answers.iter().any(|answer| !is_public_global_ip(*answer))
        {
            return Err(LookupError::Failed);
        }

        Ok(answers
            .into_iter()
            .map(|answer| SocketAddr::new(answer, 0))
            .collect())
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let lookup = Arc::clone(&self.lookup);
        let host = name.as_str().to_owned();
        #[cfg(test)]
        let connect_override = self.connect_override;
        Box::pin(async move {
            let answers = lookup.lookup(host).await?;
            let validated = Self::validate_answers(answers)?;
            #[cfg(test)]
            let validated = match connect_override {
                Some(fixture_ip) => validated
                    .into_iter()
                    .map(|answer| SocketAddr::new(fixture_ip, answer.port()))
                    .collect(),
                None => validated,
            };
            Ok(Box::new(validated.into_iter()) as Addrs)
        })
    }
}

/// Private egress primitive owned by [`super::OAuthIdentityRuntime`].
pub(super) struct OAuthEgress {
    client: reqwest::Client,
    permits: Arc<Semaphore>,
}

impl OAuthEgress {
    pub(super) fn new() -> Result<Self, OAuthRuntimeBuildError> {
        let client = build_client(GuardedResolver::system())?;
        Ok(Self {
            client,
            permits: Arc::new(Semaphore::new(MAX_OUTBOUND_REQUESTS)),
        })
    }

    /// Build the production fixed-policy client with one test trust anchor.
    ///
    /// DNS answers still pass through the production all-answer classifier;
    /// only after validation is the exact loopback fixture address substituted.
    /// This seam is absent from non-test builds and never exposes a raw client.
    #[cfg(test)]
    pub(super) fn for_test(
        trust_anchor: reqwest::Certificate,
        connect_ip: IpAddr,
        dns_answers: Vec<IpAddr>,
    ) -> Result<Self, OAuthRuntimeBuildError> {
        let resolver = GuardedResolver::for_test(
            Arc::new(StaticTestLookup {
                answers: dns_answers,
            }),
            connect_ip,
        );
        let client = build_test_client(resolver, trust_anchor)?;
        Ok(Self {
            client,
            permits: Arc::new(Semaphore::new(MAX_OUTBOUND_REQUESTS)),
        })
    }

    async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, OAuthFailureCode> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| OAuthFailureCode::CompletionTimeout)
    }

    pub(super) async fn fetch_discovery(
        &self,
        endpoint: &ServerFetchedUrl,
    ) -> Result<Zeroizing<Vec<u8>>, OAuthFailureCode> {
        let request = self
            .client
            .get(endpoint.0.clone())
            .timeout(DISCOVERY_TIMEOUT)
            .header(reqwest::header::ACCEPT, "application/json");
        self.send_limited(request, OAuthFailureCode::DiscoveryUnavailable)
            .await
    }

    pub(super) async fn exchange_token(
        &self,
        exchange: TokenExchangeRequest<'_>,
    ) -> Result<Zeroizing<Vec<u8>>, OAuthFailureCode> {
        let TokenExchangeRequest {
            endpoint,
            auth_method,
            client_id,
            client_secret,
            code,
            redirect_uri,
            code_verifier,
        } = exchange;
        let common_form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];
        let request = self
            .client
            .post(endpoint.0.clone())
            .header(reqwest::header::ACCEPT, "application/json");
        let request = match auth_method {
            TokenEndpointAuthMethod::ClientSecretBasic => {
                // RFC 6749 section 2.3.1 applies the Appendix-B
                // application/x-www-form-urlencoded component encoding to
                // each credential before joining them with the Basic colon.
                let encoded_client_id = form_urlencoded_secret(client_id.expose_secret());
                let encoded_client_secret = form_urlencoded_secret(client_secret.expose_secret());
                request
                    .basic_auth(
                        encoded_client_id.as_str(),
                        Some(encoded_client_secret.as_str()),
                    )
                    .form(&common_form)
            },
            TokenEndpointAuthMethod::ClientSecretPost => request.form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", code_verifier),
                ("client_id", client_id.expose_secret()),
                ("client_secret", client_secret.expose_secret()),
            ]),
        };
        self.send_limited(request, OAuthFailureCode::TokenExchangeFailed)
            .await
    }

    pub(super) async fn fetch_userinfo(
        &self,
        endpoint: &ServerFetchedUrl,
        access_token: &SecretString,
    ) -> Result<Zeroizing<Vec<u8>>, OAuthFailureCode> {
        let request = self
            .client
            .get(endpoint.0.clone())
            .bearer_auth(access_token.expose_secret())
            .header(reqwest::header::USER_AGENT, "nebula-api/1.0")
            .header(reqwest::header::ACCEPT, "application/json");
        self.send_limited(request, OAuthFailureCode::UserinfoFailed)
            .await
    }

    pub(super) async fn fetch_verified_email(
        &self,
        endpoint: &ServerFetchedUrl,
        access_token: &SecretString,
    ) -> Result<Zeroizing<Vec<u8>>, OAuthFailureCode> {
        let request = self
            .client
            .get(endpoint.0.clone())
            .bearer_auth(access_token.expose_secret())
            .header(reqwest::header::USER_AGENT, "nebula-api/1.0")
            .header(reqwest::header::ACCEPT, "application/json");
        self.send_limited(request, OAuthFailureCode::VerifiedEmailFailed)
            .await
    }

    async fn send_limited(
        &self,
        request: reqwest::RequestBuilder,
        failure: OAuthFailureCode,
    ) -> Result<Zeroizing<Vec<u8>>, OAuthFailureCode> {
        let _permit = self.acquire().await?;
        let response = request.send().await.map_err(|_| failure)?;
        if !response.status().is_success() {
            return Err(failure);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(failure);
        }

        let mut body = Zeroizing::new(Vec::with_capacity(MAX_RESPONSE_BYTES));
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| failure)?;
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(failure);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn form_urlencoded_secret(value: &str) -> Zeroizing<String> {
    Zeroizing::new(url::form_urlencoded::byte_serialize(value.as_bytes()).collect())
}

fn fixed_client_builder(resolver: GuardedResolver) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .tls_backend_rustls()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .referer(false)
        .connection_verbose(false)
        .no_proxy()
        .retry(reqwest::retry::never())
        .no_hickory_dns()
        .dns_resolver(resolver)
}

fn build_client(resolver: GuardedResolver) -> Result<reqwest::Client, OAuthRuntimeBuildError> {
    fixed_client_builder(resolver)
        .build()
        .map_err(|_| OAuthRuntimeBuildError::new())
}

/// Build the fixed-policy client with one ephemeral test trust anchor.
///
/// `tls_certs_only` preserves hostname/SAN verification while making the
/// trust store deterministic. There is deliberately no certificate or
/// hostname verification bypass.
#[cfg(test)]
fn build_test_client(
    resolver: GuardedResolver,
    trust_anchor: reqwest::Certificate,
) -> Result<reqwest::Client, OAuthRuntimeBuildError> {
    fixed_client_builder(resolver)
        .tls_certs_only([trust_anchor])
        .build()
        .map_err(|_| OAuthRuntimeBuildError::new())
}

/// Parsed URL that has passed the complete server-fetched endpoint policy.
///
/// Keeping validation attached to the value prevents a request site from
/// accidentally checking one string and sending another one.
#[derive(Clone)]
pub(super) struct ServerFetchedUrl(Url);

impl ServerFetchedUrl {
    pub(super) fn parse(raw: &str) -> Result<Self, OAuthFailureCode> {
        let url = validate_url_shape(raw)?;
        if url.scheme() != "https" {
            return Err(OAuthFailureCode::EndpointRejected);
        }
        validate_host(url.host())?;
        Ok(Self(url))
    }
}

impl std::fmt::Debug for ServerFetchedUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServerFetchedUrl(<redacted>)")
    }
}

/// Parsed browser authorization URL, governed by its separate redirect policy.
#[derive(Clone)]
pub(super) struct BrowserAuthorizationUrl(Url);

impl BrowserAuthorizationUrl {
    pub(super) fn parse(
        raw: &str,
        allow_insecure_localhost: bool,
        in_release_build: bool,
    ) -> Result<Self, OAuthFailureCode> {
        let url = validate_url_shape(raw)?;
        const RESERVED_QUERY_KEYS: &[&str] = &[
            "response_type",
            "client_id",
            "redirect_uri",
            "state",
            "code_challenge",
            "code_challenge_method",
            "scope",
            "nonce",
        ];
        if url.query_pairs().any(|(key, _)| {
            RESERVED_QUERY_KEYS
                .iter()
                .any(|reserved| key.eq_ignore_ascii_case(reserved))
        }) {
            return Err(OAuthFailureCode::EndpointRejected);
        }
        if url.scheme() == "https" {
            validate_host(url.host())?;
            return Ok(Self(url));
        }

        let is_localhost =
            matches!(url.host(), Some(Host::Domain(host)) if is_localhost_name(host));
        if url.scheme() == "http" && is_localhost && allow_insecure_localhost && !in_release_build {
            return Ok(Self(url));
        }
        Err(OAuthFailureCode::EndpointRejected)
    }

    pub(super) fn into_url(self) -> Url {
        self.0
    }
}

impl std::fmt::Debug for BrowserAuthorizationUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrowserAuthorizationUrl(<redacted>)")
    }
}

fn validate_url_shape(raw: &str) -> Result<Url, OAuthFailureCode> {
    let url = Url::parse(raw).map_err(|_| OAuthFailureCode::EndpointRejected)?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(OAuthFailureCode::EndpointRejected);
    }
    Ok(url)
}

fn validate_host(host: Option<Host<&str>>) -> Result<(), OAuthFailureCode> {
    match host.ok_or(OAuthFailureCode::EndpointRejected)? {
        Host::Domain(host) if is_localhost_name(host) => Err(OAuthFailureCode::EndpointRejected),
        Host::Domain(_) => Ok(()),
        Host::Ipv4(ip) if is_public_global_ip(IpAddr::V4(ip)) => Ok(()),
        Host::Ipv6(ip) if is_public_global_ip(IpAddr::V6(ip)) => Ok(()),
        Host::Ipv4(_) | Host::Ipv6(_) => Err(OAuthFailureCode::EndpointRejected),
    }
}

fn is_localhost_name(host: &str) -> bool {
    let normalized = host.trim_end_matches('.');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized.to_ascii_lowercase().ends_with(".localhost")
}

fn is_public_global_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_global_ipv4(ip),
        IpAddr::V6(ip) => is_public_global_ipv6(ip),
    }
}

fn is_public_global_ipv4(ip: Ipv4Addr) -> bool {
    const SPECIAL: &[(Ipv4Addr, u8)] = &[
        (Ipv4Addr::UNSPECIFIED, 8),
        (Ipv4Addr::new(10, 0, 0, 0), 8),
        (Ipv4Addr::new(100, 64, 0, 0), 10),
        (Ipv4Addr::new(127, 0, 0, 0), 8),
        (Ipv4Addr::new(169, 254, 0, 0), 16),
        (Ipv4Addr::new(172, 16, 0, 0), 12),
        (Ipv4Addr::new(192, 0, 0, 0), 24),
        (Ipv4Addr::new(192, 0, 2, 0), 24),
        (Ipv4Addr::new(192, 88, 99, 0), 24),
        (Ipv4Addr::new(192, 168, 0, 0), 16),
        (Ipv4Addr::new(198, 18, 0, 0), 15),
        (Ipv4Addr::new(198, 51, 100, 0), 24),
        (Ipv4Addr::new(203, 0, 113, 0), 24),
        (Ipv4Addr::new(224, 0, 0, 0), 4),
        (Ipv4Addr::new(240, 0, 0, 0), 4),
    ];

    !SPECIAL
        .iter()
        .any(|(network, prefix)| ipv4_in_prefix(ip, *network, *prefix))
}

fn is_public_global_ipv6(ip: Ipv6Addr) -> bool {
    if ip.to_ipv4_mapped().is_some() {
        return false;
    }

    const SPECIAL_WITHIN_GLOBAL_UNICAST: &[(Ipv6Addr, u8)] = &[
        (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
        (Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32),
        (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
        (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
    ];

    ipv6_in_prefix(ip, Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3)
        && !SPECIAL_WITHIN_GLOBAL_UNICAST
            .iter()
            .any(|(network, prefix)| ipv6_in_prefix(ip, *network, *prefix))
}

fn ipv4_in_prefix(ip: Ipv4Addr, network: Ipv4Addr, prefix: u8) -> bool {
    let mask = u32::MAX.checked_shl(u32::from(32 - prefix)).unwrap_or(0);
    u32::from(ip) & mask == u32::from(network) & mask
}

fn ipv6_in_prefix(ip: Ipv6Addr, network: Ipv6Addr, prefix: u8) -> bool {
    let mask = u128::MAX.checked_shl(u32::from(128 - prefix)).unwrap_or(0);
    u128::from(ip) & mask == u128::from(network) & mask
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, sync::Mutex};

    use super::*;

    struct FakeLookup {
        answers: Mutex<Option<Result<Vec<IpAddr>, LookupError>>>,
    }

    impl FakeLookup {
        fn answers(answers: Vec<IpAddr>) -> Self {
            Self {
                answers: Mutex::new(Some(Ok(answers))),
            }
        }
    }

    impl HostLookup for FakeLookup {
        fn lookup(&self, _host: String) -> LookupFuture {
            let result = self
                .answers
                .lock()
                .expect("fake lookup lock")
                .take()
                .unwrap_or(Err(LookupError::Failed));
            Box::pin(std::future::ready(result))
        }
    }

    async fn resolve_with(answers: Vec<IpAddr>) -> Result<Vec<SocketAddr>, String> {
        let resolver = GuardedResolver {
            lookup: Arc::new(FakeLookup::answers(answers)),
            connect_override: None,
        };
        let name = Name::from_str("oauth.example.com").expect("valid DNS name");
        resolver
            .resolve(name)
            .await
            .map(Iterator::collect)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn classifier_covers_iana_special_use_and_public_controls() {
        for raw in [
            "0.0.0.0",
            "10.1.2.3",
            "100.100.100.200",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:10.0.0.1",
            "64:ff9b::10.0.0.1",
            "100::1",
            "2001:db8::1",
            "2001:2::1",
            "2002:a00:1::",
            "3fff::1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
        ] {
            let ip: IpAddr = raw.parse().expect("valid table IP");
            assert!(!is_public_global_ip(ip), "special-use IP accepted: {raw}");
        }

        for raw in [
            "1.1.1.1",
            "8.8.8.8",
            "9.9.9.9",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
        ] {
            let ip: IpAddr = raw.parse().expect("valid public IP");
            assert!(is_public_global_ip(ip), "public control rejected: {raw}");
        }
    }

    #[test]
    fn url_parser_cannot_smuggle_alternate_ipv4_spellings() {
        for raw in [
            "https://2130706433/token",
            "https://0x7f000001/token",
            "https://0177.0.0.1/token",
        ] {
            assert!(
                ServerFetchedUrl::parse(raw).is_err(),
                "alternate loopback spelling accepted: {raw}"
            );
        }
    }

    #[test]
    fn server_fetched_url_rejects_non_global_ip_literals() {
        for raw in [
            "https://10.0.0.1/token",
            "https://127.0.0.1/token",
            "https://[::1]/token",
            "https://169.254.10.20/token",
            "https://224.0.0.1/token",
            "https://[fe80::1]/token",
            "https://[ff02::1]/token",
        ] {
            assert!(
                ServerFetchedUrl::parse(raw).is_err(),
                "non-global literal accepted: {raw}"
            );
        }

        for raw in [
            "https://1.1.1.1/token",
            "https://[2606:4700:4700::1111]/token",
        ] {
            assert!(
                ServerFetchedUrl::parse(raw).is_ok(),
                "public literal rejected: {raw}"
            );
        }
    }

    #[test]
    fn authorization_url_rejects_reserved_query_keys_but_keeps_vendor_keys() {
        for key in [
            "response_type",
            "client_id",
            "redirect_uri",
            "state",
            "code_challenge",
            "code_challenge_method",
            "scope",
            "nonce",
            "STATE",
        ] {
            let raw = format!("https://accounts.example.com/authorize?{key}=attacker");
            assert!(
                BrowserAuthorizationUrl::parse(&raw, false, true).is_err(),
                "reserved key accepted: {key}"
            );
        }
        assert!(
            BrowserAuthorizationUrl::parse(
                "https://accounts.example.com/authorize?tenant=customer-a",
                false,
                true,
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn resolver_rejects_private_mixed_empty_and_excessive_answers() {
        assert!(
            resolve_with(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))])
                .await
                .is_err()
        );
        assert!(
            resolve_with(vec![
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            ])
            .await
            .is_err()
        );
        assert!(resolve_with(Vec::new()).await.is_err());
        assert!(
            resolve_with(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)); 33])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn resolver_returns_only_validated_exact_public_answers() {
        let answers = vec![
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            "2606:4700:4700::1111".parse().expect("valid IPv6"),
        ];
        let resolved = resolve_with(answers.clone())
            .await
            .expect("public answers accepted");
        assert_eq!(
            resolved,
            answers
                .into_iter()
                .map(|ip| SocketAddr::new(ip, 0))
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
#[path = "egress_behavior_tests.rs"]
mod behavior_tests;
