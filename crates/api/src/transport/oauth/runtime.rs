//! Opaque semantic runtime for Plane-A OAuth identity flows.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dashmap::DashMap;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, watch};
use tokio::time::Instant;
use zeroize::Zeroizing;

use super::{
    egress::{
        BrowserAuthorizationUrl, OAuthEgress, ServerFetchedUrl, TokenEndpointAuthMethod,
        TokenExchangeRequest,
    },
    error::{OAuthFailureCode, OAuthRuntimeBuildError},
};
use crate::{
    config::{OAuthProviderConfig, OAuthProvidersConfig},
    domain::auth::backend::OAuthProvider,
};

const FLOW_DEADLINE: Duration = Duration::from_secs(30);
const DISCOVERY_TTL: Duration = Duration::from_hours(1);
const DISCOVERY_FAILURE_COOLDOWN: Duration = Duration::from_secs(5);
const GOOGLE_DISCOVERY_URL: &str = "https://accounts.google.com/.well-known/openid-configuration";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";
const GOOGLE_SCOPES: &[&str] = &["openid", "email", "profile"];
const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USERINFO_URL: &str = "https://api.github.com/user";
const GITHUB_VERIFIED_EMAILS_URL: &str = "https://api.github.com/user/emails";
const GITHUB_SCOPES: &[&str] = &["user:email"];
const ID_TOKEN_MAX_BYTES: usize = 32 * 1024;
const JWT_HEADER_MAX_BYTES: usize = 4 * 1024;
const JWT_PAYLOAD_MAX_BYTES: usize = 16 * 1024;
const JWT_SIGNATURE_MAX_BYTES: usize = 8 * 1024;
const ID_TOKEN_CLOCK_SKEW_SECS: i64 = 60;
const ID_TOKEN_MAX_AGE_SECS: i64 = 24 * 60 * 60;

/// Plane-A OAuth identity runtime.
///
/// This is the only supported owner of provider configuration, outbound HTTP,
/// DNS policy, discovery state, concurrency and deadline policy. Its fields
/// intentionally remain opaque so a backend cannot recover a raw HTTP client.
pub struct OAuthIdentityRuntime {
    providers: HashMap<OAuthProvider, RuntimeProvider>,
    egress: Arc<OAuthEgress>,
    discovery: DashMap<OAuthProvider, Arc<Mutex<DiscoveryState>>>,
}

impl std::fmt::Debug for OAuthIdentityRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthIdentityRuntime")
            .field("configured_provider_count", &self.providers.len())
            .field("egress", &"<opaque>")
            .field("discovery", &"<opaque>")
            .finish()
    }
}

impl OAuthIdentityRuntime {
    /// Build the optional fixed Plane-A runtime around an owned provider set.
    ///
    /// An empty provider set returns `None` before endpoint validation or
    /// outbound-client construction. This keeps "OAuth disabled" free of
    /// egress capabilities by construction instead of relying on every
    /// composition root to duplicate that policy.
    ///
    /// # Errors
    ///
    /// Returns a secret-free initialization error when the fixed HTTP client
    /// cannot be built or a configured endpoint violates runtime policy.
    pub fn from_config(
        providers: OAuthProvidersConfig,
    ) -> Result<Option<Self>, OAuthRuntimeBuildError> {
        if providers.providers.is_empty() {
            return Ok(None);
        }
        let providers = compile_provider_config(providers)?;
        Ok(Some(Self {
            providers,
            egress: Arc::new(OAuthEgress::new()?),
            discovery: DashMap::new(),
        }))
    }

    #[cfg(test)]
    fn from_config_with_egress(
        providers: OAuthProvidersConfig,
        profiles: HashMap<OAuthProvider, OAuthTestProviderProfile>,
        build_egress: impl FnOnce() -> Result<OAuthEgress, OAuthRuntimeBuildError>,
    ) -> Result<Option<Self>, OAuthRuntimeBuildError> {
        if providers.providers.is_empty() {
            return Ok(None);
        }
        let providers = compile_test_provider_config(providers, profiles)?;
        Ok(Some(Self {
            providers,
            egress: Arc::new(build_egress()?),
            discovery: DashMap::new(),
        }))
    }

    /// Test-only construction through the complete production URL, DNS, TLS,
    /// redirect, proxy, timeout, retry, body-cap and concurrency policy.
    #[cfg(test)]
    pub(crate) fn from_config_for_test(
        providers: OAuthProvidersConfig,
        profiles: HashMap<OAuthProvider, OAuthTestProviderProfile>,
        trust_anchor: reqwest::Certificate,
        connect_ip: std::net::IpAddr,
        dns_answers: Vec<std::net::IpAddr>,
    ) -> Result<Option<Self>, OAuthRuntimeBuildError> {
        Self::from_config_with_egress(providers, profiles, || {
            OAuthEgress::for_test(trust_anchor, connect_ip, dns_answers)
        })
    }

    /// Begin one absolute network budget. The handle is opaque and cannot be
    /// extended by a backend between the primary and fallback identity stages.
    pub(crate) fn begin_deadline(&self) -> OAuthFlowDeadline {
        OAuthFlowDeadline {
            expires_at: Instant::now() + FLOW_DEADLINE,
        }
    }

    /// Build the browser redirect for a configured provider.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            provider = provider.as_str(),
            oauth.operation = "start",
            oauth.failure_code = tracing::field::Empty
        )
    )]
    pub(crate) async fn build_authorization_url(
        &self,
        deadline: &OAuthFlowDeadline,
        provider: OAuthProvider,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> Result<String, OAuthFailureCode> {
        let result = tokio::time::timeout_at(deadline.expires_at, async {
            let provider_config = self.provider_config(provider)?;
            let endpoints = self.resolve_endpoints(provider, provider_config).await?;
            let mut url = endpoints.authorize_url.into_url();
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("response_type", "code");
                query.append_pair("client_id", provider_config.client_id.expose_secret());
                query.append_pair("redirect_uri", redirect_uri);
                query.append_pair("state", state);
                query.append_pair("code_challenge", code_challenge);
                query.append_pair("code_challenge_method", "S256");
                if provider == OAuthProvider::Google {
                    query.append_pair("nonce", &nonce_for_state(state));
                }
                if !endpoints.scopes.is_empty() {
                    query.append_pair("scope", &endpoints.scopes);
                }
            }
            Ok(url.to_string())
        })
        .await
        .unwrap_or(Err(OAuthFailureCode::CompletionTimeout));
        if let Err(code) = result.as_ref() {
            tracing::Span::current().record("oauth.failure_code", code.as_str());
        }
        result
    }

    /// Exchange the callback code and fetch primary userinfo.
    ///
    /// The returned value exposes only the stable provider subject. If a
    /// verified-email fallback may still be required, its bearer token is
    /// enclosed in a non-cloneable, non-debuggable capability.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            provider = provider.as_str(),
            oauth.operation = "complete",
            oauth.failure_code = tracing::field::Empty
        )
    )]
    pub(crate) async fn begin_identity_completion(
        &self,
        deadline: OAuthFlowDeadline,
        provider: OAuthProvider,
        state: &str,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<PendingExternalIdentity, OAuthFailureCode> {
        let expires_at = deadline.expires_at;
        let result = tokio::time::timeout_at(expires_at, async {
            let provider_config = self.provider_config(provider)?;
            let endpoints = self.resolve_endpoints(provider, provider_config).await?;
            let token_body = self
                .egress
                .exchange_token(TokenExchangeRequest {
                    endpoint: &endpoints.token_url,
                    auth_method: endpoints.token_endpoint_auth_method,
                    client_id: &provider_config.client_id,
                    client_secret: &provider_config.client_secret,
                    code,
                    redirect_uri,
                    code_verifier,
                })
                .await?;
            let token: TokenWireResponse = serde_json::from_slice(&token_body)
                .map_err(|_| OAuthFailureCode::ProviderResponseInvalid)?;
            drop(token_body);
            if !valid_access_token(token.access_token.expose_secret())
                || !token.token_type.eq_ignore_ascii_case("bearer")
            {
                return Err(OAuthFailureCode::ProviderResponseInvalid);
            }

            let expected_google_subject = match provider {
                OAuthProvider::Google => Some(validate_google_id_token(
                    token
                        .id_token
                        .as_ref()
                        .ok_or(OAuthFailureCode::ProviderResponseInvalid)?,
                    &token.access_token,
                    &provider_config.client_id,
                    &nonce_for_state(state),
                    unix_timestamp(),
                )?),
                OAuthProvider::GitHub => None,
            };

            let userinfo_body = self
                .egress
                .fetch_userinfo(&endpoints.userinfo_url, &token.access_token)
                .await?;
            let (subject, email_evidence) = match provider {
                OAuthProvider::Google => {
                    let userinfo: GoogleUserinfoWire = serde_json::from_slice(&userinfo_body)
                        .map_err(|_| OAuthFailureCode::ProviderResponseInvalid)?;
                    let subject = ValidatedExternalSubject::new(userinfo.sub)?;
                    let expected =
                        expected_google_subject.ok_or(OAuthFailureCode::ProviderResponseInvalid)?;
                    if !constant_time_equal(subject.as_str(), expected.as_str()) {
                        return Err(OAuthFailureCode::ProviderResponseInvalid);
                    }
                    (
                        subject,
                        PendingEmailEvidence::Google(GoogleEmailEvidence {
                            email: userinfo.email,
                            email_verified: userinfo.email_verified,
                            hosted_domain: userinfo.hosted_domain,
                        }),
                    )
                },
                OAuthProvider::GitHub => {
                    let userinfo: GitHubUserinfoWire = serde_json::from_slice(&userinfo_body)
                        .map_err(|_| OAuthFailureCode::ProviderResponseInvalid)?;
                    let endpoint = endpoints
                        .verified_emails_url
                        .ok_or(OAuthFailureCode::ProviderResponseInvalid)?;
                    (
                        ValidatedExternalSubject::new(userinfo.id.to_string())?,
                        PendingEmailEvidence::GitHub(VerifiedEmailCapability {
                            endpoint,
                            access_token: token.access_token,
                        }),
                    )
                },
            };
            drop(userinfo_body);

            Ok(PendingExternalIdentity {
                provider,
                subject,
                email_evidence,
                deadline,
            })
        })
        .await
        .unwrap_or(Err(OAuthFailureCode::CompletionTimeout));
        if let Err(code) = result.as_ref() {
            tracing::Span::current().record("oauth.failure_code", code.as_str());
        }
        result
    }

    /// Consume a pending identity and resolve its verified email when needed.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            provider = pending.provider.as_str(),
            oauth.operation = "complete_verified_email",
            oauth.failure_code = tracing::field::Empty
        )
    )]
    pub(crate) async fn resolve_verified_identity(
        &self,
        pending: PendingExternalIdentity,
    ) -> Result<ProvisionableEmail, OAuthFailureCode> {
        let expires_at = pending.deadline.expires_at;
        let result = tokio::time::timeout_at(expires_at, async {
            let PendingExternalIdentity {
                provider: _,
                subject,
                email_evidence,
                deadline: _,
            } = pending;

            let verified_email = match email_evidence {
                PendingEmailEvidence::Google(evidence) => validate_google_email(evidence)?,
                PendingEmailEvidence::GitHub(capability) => {
                    self.consume_verified_email(capability).await?
                },
            };

            drop(subject);
            Ok(verified_email)
        })
        .await
        .unwrap_or(Err(OAuthFailureCode::CompletionTimeout));
        if let Err(code) = result.as_ref() {
            tracing::Span::current().record("oauth.failure_code", code.as_str());
        }
        result
    }

    fn provider_config(
        &self,
        provider: OAuthProvider,
    ) -> Result<&RuntimeProvider, OAuthFailureCode> {
        self.providers
            .get(&provider)
            .ok_or(OAuthFailureCode::ProviderNotConfigured)
    }

    async fn consume_verified_email(
        &self,
        capability: VerifiedEmailCapability,
    ) -> Result<ProvisionableEmail, OAuthFailureCode> {
        let body = self
            .egress
            .fetch_verified_email(&capability.endpoint, &capability.access_token)
            .await?;
        let entries: Vec<VerifiedEmailWireEntry> =
            serde_json::from_slice(&body).map_err(|_| OAuthFailureCode::ProviderResponseInvalid)?;
        drop(body);
        entries
            .into_iter()
            .find(|entry| entry.primary && entry.verified && !entry.email.is_empty())
            .map(|entry| ProvisionableEmail::new(entry.email))
            .transpose()?
            .ok_or(OAuthFailureCode::VerifiedEmailUnavailable)
    }

    async fn resolve_endpoints(
        &self,
        provider: OAuthProvider,
        config: &RuntimeProvider,
    ) -> Result<ResolvedEndpoints, OAuthFailureCode> {
        match &config.endpoints {
            RuntimeEndpoints::Resolved(endpoints) => Ok(endpoints.as_ref().clone()),
            RuntimeEndpoints::Oidc { discovery_url } => {
                self.resolve_discovered_endpoints(provider, discovery_url)
                    .await
            },
        }
    }

    async fn resolve_discovered_endpoints(
        &self,
        provider: OAuthProvider,
        endpoint: &ServerFetchedUrl,
    ) -> Result<ResolvedEndpoints, OAuthFailureCode> {
        let slot = {
            let entry = self
                .discovery
                .entry(provider)
                .or_insert_with(|| Arc::new(Mutex::new(DiscoveryState::default())));
            Arc::clone(entry.value())
        };

        // Elect or join a background-owned flight under a short lock. The
        // request that happened to initiate discovery does not own the fetch:
        // cancellation and per-caller deadlines only drop that caller's
        // receiver, while the cache-or-cooldown transition still completes.
        let mut receiver = {
            let mut state = slot.lock().await;
            let now = Instant::now();
            if let Some(cached) = state.cached.as_ref()
                && cached.expires_at > now
            {
                return Ok(cached.endpoints.clone());
            }
            if state
                .retry_not_before
                .is_some_and(|retry_at| retry_at > now)
            {
                return Err(OAuthFailureCode::DiscoveryUnavailable);
            }
            if let Some(receiver) = state.in_flight.as_ref() {
                receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                state.in_flight = Some(receiver.clone());
                let egress = Arc::clone(&self.egress);
                let endpoint = endpoint.clone();
                let slot = Arc::clone(&slot);
                tokio::spawn(async move {
                    let result = fetch_discovered_endpoints(&egress, &endpoint).await;
                    publish_discovery_result(&slot, &result).await;
                    let _ = sender.send(Some(result));
                });
                receiver
            }
        };

        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| OAuthFailureCode::DiscoveryUnavailable)?;
        }
    }
}

async fn publish_discovery_result(
    slot: &Mutex<DiscoveryState>,
    result: &Result<ResolvedEndpoints, OAuthFailureCode>,
) {
    let completed_at = Instant::now();
    let mut state = slot.lock().await;
    if let Ok(endpoints) = result {
        state.cached = Some(CachedDiscovery {
            endpoints: endpoints.clone(),
            expires_at: completed_at + DISCOVERY_TTL,
        });
        state.retry_not_before = None;
    } else {
        state.cached = None;
        state.retry_not_before = Some(completed_at + DISCOVERY_FAILURE_COOLDOWN);
    }
    state.in_flight = None;
}

async fn fetch_discovered_endpoints(
    egress: &OAuthEgress,
    endpoint: &ServerFetchedUrl,
) -> Result<ResolvedEndpoints, OAuthFailureCode> {
    let body = egress.fetch_discovery(endpoint).await?;
    let discovery: OidcDiscoveryWire =
        serde_json::from_slice(&body).map_err(|_| OAuthFailureCode::DiscoveryUnavailable)?;
    drop(body);

    if discovery.issuer != GOOGLE_ISSUER {
        return Err(OAuthFailureCode::DiscoveryUnavailable);
    }
    // Validate every child before publishing the cache entry, including
    // the currently-unused JWKS URL.
    if let Some(jwks_url) = discovery.jwks.as_deref() {
        ServerFetchedUrl::parse(jwks_url)?;
    }
    Ok(ResolvedEndpoints {
        authorize_url: BrowserAuthorizationUrl::parse(&discovery.authorization, false, true)?,
        token_url: ServerFetchedUrl::parse(&discovery.token)?,
        token_endpoint_auth_method: select_discovered_token_auth_method(
            discovery.token_endpoint_auth_methods_supported.as_deref(),
        )?,
        userinfo_url: ServerFetchedUrl::parse(&discovery.userinfo)?,
        verified_emails_url: None,
        scopes: GOOGLE_SCOPES.join(" "),
    })
}

/// Opaque result of token exchange plus primary userinfo.
///
/// Deliberately not `Clone` or `Debug`: the optional capability owns a bearer
/// token and can only be consumed by [`OAuthIdentityRuntime`].
pub(crate) struct PendingExternalIdentity {
    provider: OAuthProvider,
    subject: ValidatedExternalSubject,
    email_evidence: PendingEmailEvidence,
    deadline: OAuthFlowDeadline,
}

impl PendingExternalIdentity {
    pub(crate) fn subject(&self) -> &str {
        self.subject.as_str()
    }
}

struct ValidatedExternalSubject(String);

impl ValidatedExternalSubject {
    fn new(subject: String) -> Result<Self, OAuthFailureCode> {
        validate_subject(&subject)?;
        Ok(Self(subject))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) struct ProvisionableEmail(String);

impl ProvisionableEmail {
    fn new(email: String) -> Result<Self, OAuthFailureCode> {
        normalize_verified_email(email).map(Self)
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

enum PendingEmailEvidence {
    Google(GoogleEmailEvidence),
    GitHub(VerifiedEmailCapability),
}

struct GoogleEmailEvidence {
    email: Option<String>,
    email_verified: Option<bool>,
    hosted_domain: Option<String>,
}

struct VerifiedEmailCapability {
    endpoint: ServerFetchedUrl,
    access_token: SecretString,
}

pub(crate) struct OAuthFlowDeadline {
    expires_at: Instant,
}

struct RuntimeProvider {
    client_id: SecretString,
    client_secret: SecretString,
    endpoints: RuntimeEndpoints,
}

enum RuntimeEndpoints {
    Oidc { discovery_url: ServerFetchedUrl },
    Resolved(Box<ResolvedEndpoints>),
}

/// Explicit endpoint seam for hermetic runtime tests. It is crate-private and
/// does not exist in non-test builds, so neither serde nor environment config
/// can enable it.
#[cfg(test)]
pub(crate) enum OAuthTestProviderProfile {
    Oidc {
        discovery_url: String,
    },
    Manual {
        authorize_url: String,
        token_url: String,
        userinfo_url: String,
        verified_emails_url: Option<String>,
        scopes: Vec<String>,
    },
}

#[cfg(test)]
impl OAuthTestProviderProfile {
    pub(crate) fn oidc(discovery_url: String) -> Self {
        Self::Oidc { discovery_url }
    }

    pub(crate) fn manual(
        authorize_url: String,
        token_url: String,
        userinfo_url: String,
        verified_emails_url: Option<String>,
        scopes: Vec<String>,
    ) -> Self {
        Self::Manual {
            authorize_url,
            token_url,
            userinfo_url,
            verified_emails_url,
            scopes,
        }
    }
}

#[derive(Clone)]
struct ResolvedEndpoints {
    authorize_url: BrowserAuthorizationUrl,
    token_url: ServerFetchedUrl,
    token_endpoint_auth_method: TokenEndpointAuthMethod,
    userinfo_url: ServerFetchedUrl,
    verified_emails_url: Option<ServerFetchedUrl>,
    scopes: String,
}

#[derive(Default)]
struct DiscoveryState {
    cached: Option<CachedDiscovery>,
    retry_not_before: Option<Instant>,
    in_flight: Option<watch::Receiver<Option<Result<ResolvedEndpoints, OAuthFailureCode>>>>,
}

struct CachedDiscovery {
    endpoints: ResolvedEndpoints,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct OidcDiscoveryWire {
    issuer: String,
    #[serde(rename = "authorization_endpoint")]
    authorization: String,
    #[serde(rename = "token_endpoint")]
    token: String,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
    #[serde(rename = "userinfo_endpoint")]
    userinfo: String,
    #[serde(default, rename = "jwks_uri")]
    jwks: Option<String>,
}

#[derive(Deserialize)]
struct TokenWireResponse {
    access_token: SecretString,
    token_type: String,
    #[serde(default)]
    id_token: Option<SecretString>,
}

#[derive(Deserialize)]
struct GoogleUserinfoWire {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<bool>,
    #[serde(default, rename = "hd")]
    hosted_domain: Option<String>,
}

#[derive(Deserialize)]
struct GitHubUserinfoWire {
    id: u64,
}

#[derive(Deserialize)]
struct VerifiedEmailWireEntry {
    email: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    verified: bool,
}

#[derive(Deserialize)]
struct GoogleIdTokenHeader {
    alg: String,
    #[serde(default)]
    typ: Option<String>,
}

#[derive(Deserialize)]
struct GoogleIdTokenClaims {
    iss: String,
    sub: String,
    aud: JwtAudience,
    #[serde(default)]
    azp: Option<String>,
    exp: i64,
    iat: i64,
    nonce: String,
    at_hash: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JwtAudience {
    One(String),
    Many(Vec<String>),
}

impl JwtAudience {
    fn values(&self) -> &[String] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
}

fn nonce_for_state(state: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nebula-google-oauth-nonce-v1\0");
    digest.update(state.as_bytes());
    URL_SAFE_NO_PAD.encode(digest.finalize())
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    left.len() == right.len() && bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

fn decode_jwt_segment(
    encoded: &str,
    max_decoded_bytes: usize,
) -> Result<Zeroizing<Vec<u8>>, OAuthFailureCode> {
    if encoded.is_empty() || encoded.len() > max_decoded_bytes.saturating_mul(2) {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    let decoded = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| OAuthFailureCode::ProviderResponseInvalid)?,
    );
    if decoded.is_empty() || decoded.len() > max_decoded_bytes {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    Ok(decoded)
}

fn expected_oidc_at_hash(access_token: &str) -> String {
    let digest = Sha256::digest(access_token.as_bytes());
    URL_SAFE_NO_PAD.encode(&digest[..digest.len() / 2])
}

fn validate_google_id_token(
    id_token: &SecretString,
    access_token: &SecretString,
    client_id: &SecretString,
    expected_nonce: &str,
    now: i64,
) -> Result<ValidatedExternalSubject, OAuthFailureCode> {
    let compact = id_token.expose_secret();
    if compact.is_empty() || compact.len() > ID_TOKEN_MAX_BYTES {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    let mut segments = compact.split('.');
    let header = segments
        .next()
        .ok_or(OAuthFailureCode::ProviderResponseInvalid)?;
    let payload = segments
        .next()
        .ok_or(OAuthFailureCode::ProviderResponseInvalid)?;
    let signature = segments
        .next()
        .ok_or(OAuthFailureCode::ProviderResponseInvalid)?;
    if segments.next().is_some() {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }

    let header = decode_jwt_segment(header, JWT_HEADER_MAX_BYTES)?;
    let payload = decode_jwt_segment(payload, JWT_PAYLOAD_MAX_BYTES)?;
    // Core §3.1.3.7 permits this direct TLS token-endpoint validation path;
    // signature bytes are syntax/bounds checked but deliberately not treated
    // as a JWKS-validated assertion. The raw and decoded material zeroize.
    let _signature = decode_jwt_segment(signature, JWT_SIGNATURE_MAX_BYTES)?;
    let header: GoogleIdTokenHeader =
        serde_json::from_slice(&header).map_err(|_| OAuthFailureCode::ProviderResponseInvalid)?;
    let claims: GoogleIdTokenClaims =
        serde_json::from_slice(&payload).map_err(|_| OAuthFailureCode::ProviderResponseInvalid)?;

    if header.alg != "RS256" || header.typ.as_deref().is_some_and(|typ| typ != "JWT") {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    if claims.iss != GOOGLE_ISSUER {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    let audience = claims.aud.values();
    let client_id = client_id.expose_secret();
    // The built-in Google profile deliberately narrows Core's multi-audience
    // allowance: every audience entry must be this one Nebula client. Nebula
    // has no multi-party token use case, so co-audience tokens fail closed
    // even when `azp` names this client.
    if audience.is_empty()
        || audience.len() > 4
        || audience
            .iter()
            .any(|candidate| !constant_time_equal(candidate, client_id))
        || (audience.len() > 1 && claims.azp.is_none())
        || claims
            .azp
            .as_deref()
            .is_some_and(|azp| !constant_time_equal(azp, client_id))
    {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    let earliest_valid = now.saturating_sub(ID_TOKEN_CLOCK_SKEW_SECS);
    let latest_expiry = now
        .saturating_add(ID_TOKEN_MAX_AGE_SECS)
        .saturating_add(ID_TOKEN_CLOCK_SKEW_SECS);
    let latest_issued_at = now.saturating_add(ID_TOKEN_CLOCK_SKEW_SECS);
    let earliest_issued_at = now.saturating_sub(ID_TOKEN_MAX_AGE_SECS);
    if claims.exp < earliest_valid
        || claims.exp > latest_expiry
        || claims.iat < earliest_issued_at
        || claims.iat > latest_issued_at
        || claims.iat > claims.exp
        || !constant_time_equal(&claims.nonce, expected_nonce)
    {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    let expected_at_hash = expected_oidc_at_hash(access_token.expose_secret());
    if !constant_time_equal(&claims.at_hash, &expected_at_hash) {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }

    ValidatedExternalSubject::new(claims.sub)
}

fn validate_google_email(
    evidence: GoogleEmailEvidence,
) -> Result<ProvisionableEmail, OAuthFailureCode> {
    if evidence.email_verified != Some(true) {
        return Err(OAuthFailureCode::VerifiedEmailUnavailable);
    }
    let email = ProvisionableEmail::new(
        evidence
            .email
            .ok_or(OAuthFailureCode::VerifiedEmailUnavailable)?,
    )?;
    let domain = email
        .0
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .ok_or(OAuthFailureCode::ProviderResponseInvalid)?;
    if domain == "gmail.com" {
        return Ok(email);
    }
    let hosted_domain = evidence
        .hosted_domain
        .map(|domain| domain.trim().to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .ok_or(OAuthFailureCode::VerifiedEmailUnavailable)?;
    if hosted_domain != domain {
        return Err(OAuthFailureCode::VerifiedEmailUnavailable);
    }
    Ok(email)
}

fn select_discovered_token_auth_method(
    methods: Option<&[String]>,
) -> Result<TokenEndpointAuthMethod, OAuthFailureCode> {
    let Some(methods) = methods else {
        return Ok(TokenEndpointAuthMethod::ClientSecretBasic);
    };
    if methods.iter().any(|method| method == "client_secret_basic") {
        Ok(TokenEndpointAuthMethod::ClientSecretBasic)
    } else if methods.iter().any(|method| method == "client_secret_post") {
        Ok(TokenEndpointAuthMethod::ClientSecretPost)
    } else {
        Err(OAuthFailureCode::DiscoveryUnavailable)
    }
}

fn valid_access_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 16 * 1024
        && token.trim() == token
        && !token.chars().any(char::is_whitespace)
        && !token.bytes().any(|byte| byte.is_ascii_control())
}

fn normalize_verified_email(email: String) -> Result<String, OAuthFailureCode> {
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty()
        || normalized.len() > 254
        || normalized.chars().any(char::is_whitespace)
        || normalized.chars().any(char::is_control)
    {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    let mut parts = normalized.split('@');
    let Some(local) = parts.next() else {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    };
    let Some(domain) = parts.next() else {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    };
    let local_valid = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..");
    let domain_valid = !domain.is_empty()
        && parts.next().is_none()
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !local_valid || !domain_valid {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    Ok(normalized)
}

fn validate_subject(subject: &str) -> Result<(), OAuthFailureCode> {
    if subject.is_empty()
        || subject.len() > 255
        || subject.trim() != subject
        || subject.chars().any(char::is_control)
    {
        return Err(OAuthFailureCode::ProviderResponseInvalid);
    }
    Ok(())
}

fn compile_provider_config(
    config: OAuthProvidersConfig,
) -> Result<HashMap<OAuthProvider, RuntimeProvider>, OAuthRuntimeBuildError> {
    let OAuthProvidersConfig { providers } = config;
    let mut compiled = HashMap::with_capacity(providers.len());
    for (provider, provider_config) in providers {
        let OAuthProviderConfig {
            client_id,
            client_secret,
        } = provider_config;
        if client_id.expose_secret().is_empty() || client_secret.expose_secret().is_empty() {
            return Err(OAuthRuntimeBuildError::new());
        }
        let endpoints = match provider {
            OAuthProvider::Google => RuntimeEndpoints::Oidc {
                discovery_url: parse_server_url(GOOGLE_DISCOVERY_URL)?,
            },
            OAuthProvider::GitHub => fixed_github_endpoints()?,
        };
        compiled.insert(
            provider,
            RuntimeProvider {
                client_id,
                client_secret,
                endpoints,
            },
        );
    }
    Ok(compiled)
}

fn parse_server_url(raw: &str) -> Result<ServerFetchedUrl, OAuthRuntimeBuildError> {
    ServerFetchedUrl::parse(raw).map_err(|_| OAuthRuntimeBuildError::new())
}

fn fixed_github_endpoints() -> Result<RuntimeEndpoints, OAuthRuntimeBuildError> {
    Ok(RuntimeEndpoints::Resolved(Box::new(ResolvedEndpoints {
        authorize_url: BrowserAuthorizationUrl::parse(GITHUB_AUTHORIZE_URL, false, true)
            .map_err(|_| OAuthRuntimeBuildError::new())?,
        token_url: parse_server_url(GITHUB_TOKEN_URL)?,
        token_endpoint_auth_method: TokenEndpointAuthMethod::ClientSecretPost,
        userinfo_url: parse_server_url(GITHUB_USERINFO_URL)?,
        verified_emails_url: Some(parse_server_url(GITHUB_VERIFIED_EMAILS_URL)?),
        scopes: GITHUB_SCOPES.join(" "),
    })))
}

#[cfg(test)]
fn compile_test_provider_config(
    config: OAuthProvidersConfig,
    mut profiles: HashMap<OAuthProvider, OAuthTestProviderProfile>,
) -> Result<HashMap<OAuthProvider, RuntimeProvider>, OAuthRuntimeBuildError> {
    let OAuthProvidersConfig { providers } = config;
    if profiles.len() != providers.len() {
        return Err(OAuthRuntimeBuildError::new());
    }
    let mut compiled = HashMap::with_capacity(providers.len());
    for (provider, provider_config) in providers {
        let OAuthProviderConfig {
            client_id,
            client_secret,
        } = provider_config;
        if client_id.expose_secret().is_empty() || client_secret.expose_secret().is_empty() {
            return Err(OAuthRuntimeBuildError::new());
        }
        let profile = profiles
            .remove(&provider)
            .ok_or_else(OAuthRuntimeBuildError::new)?;
        let endpoints = match profile {
            OAuthTestProviderProfile::Oidc { discovery_url } => RuntimeEndpoints::Oidc {
                discovery_url: parse_server_url(&discovery_url)?,
            },
            OAuthTestProviderProfile::Manual {
                authorize_url,
                token_url,
                userinfo_url,
                verified_emails_url,
                scopes,
            } => {
                if scopes.is_empty() {
                    return Err(OAuthRuntimeBuildError::new());
                }
                RuntimeEndpoints::Resolved(Box::new(ResolvedEndpoints {
                    authorize_url: BrowserAuthorizationUrl::parse(&authorize_url, false, true)
                        .map_err(|_| OAuthRuntimeBuildError::new())?,
                    token_url: parse_server_url(&token_url)?,
                    token_endpoint_auth_method: TokenEndpointAuthMethod::ClientSecretPost,
                    userinfo_url: parse_server_url(&userinfo_url)?,
                    verified_emails_url: verified_emails_url
                        .as_deref()
                        .map(parse_server_url)
                        .transpose()?,
                    scopes: scopes.join(" "),
                }))
            },
        };
        compiled.insert(
            provider,
            RuntimeProvider {
                client_id,
                client_secret,
                endpoints,
            },
        );
    }
    if !profiles.is_empty() {
        return Err(OAuthRuntimeBuildError::new());
    }
    Ok(compiled)
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
