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
mod tests {
    use std::{
        collections::HashMap,
        net::{IpAddr, Ipv4Addr},
        sync::Arc,
        time::Duration,
    };

    use super::*;
    use crate::transport::oauth::test_support::{TestResponse, TlsFixture};

    const TEST_DNS_ANSWER: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));

    type TestRuntimeConfig = (
        OAuthProvidersConfig,
        HashMap<OAuthProvider, OAuthTestProviderProfile>,
    );

    fn provider_config(provider: OAuthProvider, client_id: &str) -> OAuthProvidersConfig {
        OAuthProvidersConfig {
            providers: HashMap::from([(
                provider,
                OAuthProviderConfig {
                    client_id: SecretString::new(client_id.to_owned().into_boxed_str()),
                    client_secret: SecretString::new("test-secret".to_owned().into_boxed_str()),
                },
            )]),
        }
    }

    fn manual_config(fixture: &TlsFixture) -> TestRuntimeConfig {
        (
            provider_config(OAuthProvider::GitHub, "test-client"),
            HashMap::from([(
                OAuthProvider::GitHub,
                OAuthTestProviderProfile::manual(
                    "https://accounts.example.com/authorize".to_owned(),
                    fixture.endpoint("/token"),
                    fixture.endpoint("/userinfo"),
                    Some(fixture.endpoint("/emails")),
                    vec!["user:email".to_owned()],
                ),
            )]),
        )
    }

    fn oidc_config(fixture: &TlsFixture) -> TestRuntimeConfig {
        (
            provider_config(OAuthProvider::Google, "test-client"),
            HashMap::from([(
                OAuthProvider::Google,
                OAuthTestProviderProfile::oidc(fixture.endpoint("/discovery")),
            )]),
        )
    }

    fn google_manual_test_config(fixture: &TlsFixture) -> TestRuntimeConfig {
        (
            provider_config(OAuthProvider::Google, "google-client-id"),
            HashMap::from([(
                OAuthProvider::Google,
                OAuthTestProviderProfile::manual(
                    "https://accounts.example.com/authorize".to_owned(),
                    fixture.endpoint("/token"),
                    fixture.endpoint("/userinfo"),
                    None,
                    GOOGLE_SCOPES
                        .iter()
                        .map(|scope| (*scope).to_owned())
                        .collect(),
                ),
            )]),
        )
    }

    fn runtime_for_fixture(
        (config, profiles): TestRuntimeConfig,
        fixture: &TlsFixture,
    ) -> OAuthIdentityRuntime {
        OAuthIdentityRuntime::from_config_for_test(
            config,
            profiles,
            fixture.trust_anchor(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            vec![TEST_DNS_ANSWER],
        )
        .expect("fixed-policy test runtime must build")
        .expect("non-empty provider config must enable OAuth")
    }

    async fn wait_for_discovery_state(
        slot: &Arc<Mutex<DiscoveryState>>,
        ready: impl Fn(&DiscoveryState) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let is_ready = {
                    let state = slot.lock().await;
                    ready(&state)
                };
                if is_ready {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("background discovery flight published its terminal state");
    }

    #[test]
    fn empty_config_disables_oauth_before_egress_construction() {
        let runtime = OAuthIdentityRuntime::from_config_with_egress(
            OAuthProvidersConfig::default(),
            HashMap::new(),
            || panic!("empty provider config must not construct outbound egress"),
        )
        .expect("empty provider config is a valid disabled state");

        assert!(runtime.is_none());
    }

    #[test]
    fn runtime_debug_exposes_only_policy_shape() {
        const CLIENT_CANARY: &str = "CLIENT_CANARY_DO_NOT_LOG";
        const SECRET_CANARY: &str = "SECRET_CANARY_DO_NOT_LOG";
        let providers = HashMap::from([(
            OAuthProvider::GitHub,
            OAuthProviderConfig {
                client_id: SecretString::new(CLIENT_CANARY.to_owned().into_boxed_str()),
                client_secret: SecretString::new(SECRET_CANARY.to_owned().into_boxed_str()),
            },
        )]);
        let runtime = OAuthIdentityRuntime::from_config(OAuthProvidersConfig { providers })
            .expect("valid runtime config must build")
            .expect("non-empty config must enable OAuth");

        let debug = format!("{runtime:?}");
        assert!(debug.contains("configured_provider_count: 1"));
        for canary in [CLIENT_CANARY, SECRET_CANARY] {
            assert!(!debug.contains(canary), "runtime Debug leaked {canary}");
        }
    }

    #[tokio::test]
    async fn identity_truth_table_uses_only_attested_email_sources() {
        let inline = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
            "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
            "/userinfo" => TestResponse::json(r#"{"id":41,"email":"unattested@example.com"}"#),
            "/emails" => TestResponse::json(
                r#"[{"email":" User@Example.COM ","primary":true,"verified":true}]"#,
            ),
            _ => TestResponse::failure(404),
        })
        .await;
        let runtime = runtime_for_fixture(manual_config(&inline), &inline);
        let pending = runtime
            .begin_identity_completion(
                runtime.begin_deadline(),
                OAuthProvider::GitHub,
                "state",
                "code",
                "https://nebula.example/api/v1/auth/oauth/github/callback",
                "verifier",
            )
            .await
            .expect("inline verified identity must resolve");
        assert_eq!(pending.subject(), "41");
        assert_eq!(
            runtime
                .resolve_verified_identity(pending)
                .await
                .expect("GitHub verified email must normalize")
                .into_string(),
            "user@example.com"
        );
        assert_eq!(
            inline
                .requests()
                .into_iter()
                .map(|request| request.path)
                .collect::<Vec<_>>(),
            ["/token", "/userinfo", "/emails"]
        );

        let fallback = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
            "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"bearer"}"#),
            "/userinfo" => TestResponse::json(r#"{"id":42,"email":"unattested@example.com"}"#),
            "/emails" => TestResponse::json(
                r#"[{"email":"other@example.com","primary":false,"verified":true},{"email":"Primary@Example.COM","primary":true,"verified":true}]"#,
            ),
            _ => TestResponse::failure(404),
        })
        .await;
        let runtime = runtime_for_fixture(manual_config(&fallback), &fallback);
        let pending = runtime
            .begin_identity_completion(
                runtime.begin_deadline(),
                OAuthProvider::GitHub,
                "state",
                "code",
                "https://nebula.example/api/v1/auth/oauth/github/callback",
                "verifier",
            )
            .await
            .expect("fallback identity must resolve");
        assert_eq!(pending.subject(), "42");
        assert_eq!(
            runtime
                .resolve_verified_identity(pending)
                .await
                .expect("primary verified fallback must resolve")
                .into_string(),
            "primary@example.com"
        );
        assert_eq!(
            fallback
                .requests()
                .into_iter()
                .map(|request| request.path)
                .collect::<Vec<_>>(),
            ["/token", "/userinfo", "/emails"]
        );

        let rejected = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
            "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
            "/userinfo" => TestResponse::json(r#"{"id":43}"#),
            "/emails" => TestResponse::json(
                r#"[{"email":"rejected@example.com","primary":false,"verified":true}]"#,
            ),
            _ => TestResponse::failure(404),
        })
        .await;
        let runtime = runtime_for_fixture(manual_config(&rejected), &rejected);
        let pending = runtime
            .begin_identity_completion(
                runtime.begin_deadline(),
                OAuthProvider::GitHub,
                "state",
                "code",
                "https://nebula.example/api/v1/auth/oauth/github/callback",
                "verifier",
            )
            .await
            .expect("userinfo response itself is structurally valid");
        let error = match runtime.resolve_verified_identity(pending).await {
            Ok(_) => panic!("non-primary GitHub email is not signup evidence"),
            Err(error) => error,
        };
        assert_eq!(error, OAuthFailureCode::VerifiedEmailUnavailable);
        assert_eq!(
            rejected
                .requests()
                .into_iter()
                .map(|request| request.path)
                .collect::<Vec<_>>(),
            ["/token", "/userinfo", "/emails"]
        );
    }

    #[tokio::test]
    async fn discovery_singleflight_cache_and_one_hour_ttl_are_enforced() {
        let discovery = TlsFixture::spawn("oauth.test", |request, _| {
            assert_eq!(request.path, "/discovery");
            TestResponse::json(
                r#"{"issuer":"https://accounts.google.com","authorization_endpoint":"https://accounts.example.com/authorize","token_endpoint":"https://token.example.com/token","userinfo_endpoint":"https://userinfo.example.com/user","jwks_uri":"https://keys.example.com/jwks"}"#,
            )
            .delayed(Duration::from_millis(75))
        })
        .await;
        let runtime = Arc::new(runtime_for_fixture(oidc_config(&discovery), &discovery));
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..16 {
            let runtime = Arc::clone(&runtime);
            tasks.spawn(async move {
                let deadline = runtime.begin_deadline();
                runtime
                    .build_authorization_url(
                        &deadline,
                        OAuthProvider::Google,
                        "https://nebula.example/api/v1/auth/oauth/google/callback",
                        &format!("state-{index}"),
                        &format!("challenge-{index}"),
                    )
                    .await
            });
        }
        while let Some(result) = tasks.join_next().await {
            result
                .expect("singleflight task must join")
                .expect("every follower must receive discovered endpoints");
        }
        assert_eq!(
            discovery.requests().len(),
            1,
            "followers must share one fetch"
        );

        let slot = Arc::clone(
            runtime
                .discovery
                .get(&OAuthProvider::Google)
                .expect("provider cache slot must exist")
                .value(),
        );
        let mut state = slot.lock().await;
        let expires_at = state
            .cached
            .as_ref()
            .expect("successful discovery must be cached")
            .expires_at;
        let remaining = expires_at.saturating_duration_since(Instant::now());
        assert!(remaining <= DISCOVERY_TTL);
        assert!(remaining > Duration::from_mins(59));
        state
            .cached
            .as_mut()
            .expect("cache entry remains present")
            .expires_at = Instant::now() - Duration::from_millis(1);
        drop(state);

        let deadline = runtime.begin_deadline();
        runtime
            .build_authorization_url(
                &deadline,
                OAuthProvider::Google,
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "state-after-expiry",
                "challenge-after-expiry",
            )
            .await
            .expect("expired discovery cache must refetch");
        assert_eq!(discovery.requests().len(), 2);
    }

    #[tokio::test]
    async fn discovery_flight_survives_initiator_abort_and_serves_follower() {
        let discovery = TlsFixture::spawn("oauth.test", |_request, _| {
            TestResponse::json(
                r#"{"issuer":"https://accounts.google.com","authorization_endpoint":"https://accounts.example.com/authorize","token_endpoint":"https://token.example.com/token","userinfo_endpoint":"https://userinfo.example.com/user"}"#,
            )
            .delayed(Duration::from_millis(100))
        })
        .await;
        let runtime = Arc::new(runtime_for_fixture(oidc_config(&discovery), &discovery));
        let initiating_runtime = Arc::clone(&runtime);
        let initiating = tokio::spawn(async move {
            let deadline = initiating_runtime.begin_deadline();
            initiating_runtime
                .build_authorization_url(
                    &deadline,
                    OAuthProvider::Google,
                    "https://nebula.example/api/v1/auth/oauth/google/callback",
                    "initiator-state",
                    "initiator-challenge",
                )
                .await
        });
        discovery.wait_for_request_count(1).await;
        initiating.abort();
        assert!(
            initiating
                .await
                .expect_err("initiating caller must be cancelled")
                .is_cancelled()
        );

        let follower_deadline = runtime.begin_deadline();
        runtime
            .build_authorization_url(
                &follower_deadline,
                OAuthProvider::Google,
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "follower-state",
                "follower-challenge",
            )
            .await
            .expect("follower must receive the background-owned flight result");
        assert_eq!(discovery.requests().len(), 1);
    }

    #[tokio::test]
    async fn discovery_flight_completes_cache_after_every_caller_cancels() {
        let discovery = TlsFixture::spawn("oauth.test", |_request, _| {
            TestResponse::json(
                r#"{"issuer":"https://accounts.google.com","authorization_endpoint":"https://accounts.example.com/authorize","token_endpoint":"https://token.example.com/token","userinfo_endpoint":"https://userinfo.example.com/user"}"#,
            )
            .delayed(Duration::from_millis(100))
        })
        .await;
        let runtime = Arc::new(runtime_for_fixture(oidc_config(&discovery), &discovery));
        let mut callers = Vec::new();
        for index in 0..4 {
            let runtime = Arc::clone(&runtime);
            callers.push(tokio::spawn(async move {
                let deadline = runtime.begin_deadline();
                runtime
                    .build_authorization_url(
                        &deadline,
                        OAuthProvider::Google,
                        "https://nebula.example/api/v1/auth/oauth/google/callback",
                        &format!("cancelled-state-{index}"),
                        &format!("cancelled-challenge-{index}"),
                    )
                    .await
            }));
        }
        discovery.wait_for_request_count(1).await;
        for caller in callers {
            caller.abort();
        }

        let slot = Arc::clone(
            runtime
                .discovery
                .get(&OAuthProvider::Google)
                .expect("provider cache slot must exist")
                .value(),
        );
        wait_for_discovery_state(&slot, |state| {
            state.cached.is_some() && state.in_flight.is_none()
        })
        .await;

        let later_deadline = runtime.begin_deadline();
        runtime
            .build_authorization_url(
                &later_deadline,
                OAuthProvider::Google,
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "later-state",
                "later-challenge",
            )
            .await
            .expect("later caller must use the completed cache");
        assert_eq!(discovery.requests().len(), 1);
    }

    #[tokio::test]
    async fn timed_out_discovery_caller_still_installs_failure_cooldown() {
        let discovery = TlsFixture::spawn("oauth.test", |_request, _| {
            TestResponse::failure(503).delayed(Duration::from_millis(100))
        })
        .await;
        let runtime = runtime_for_fixture(oidc_config(&discovery), &discovery);
        let short_deadline = OAuthFlowDeadline {
            expires_at: Instant::now() + Duration::from_millis(20),
        };
        assert_eq!(
            runtime
                .build_authorization_url(
                    &short_deadline,
                    OAuthProvider::Google,
                    "https://nebula.example/api/v1/auth/oauth/google/callback",
                    "timed-out-state",
                    "timed-out-challenge",
                )
                .await
                .expect_err("caller's own deadline must still apply"),
            OAuthFailureCode::CompletionTimeout
        );

        let slot = Arc::clone(
            runtime
                .discovery
                .get(&OAuthProvider::Google)
                .expect("provider cooldown slot must exist")
                .value(),
        );
        wait_for_discovery_state(&slot, |state| {
            state.retry_not_before.is_some() && state.in_flight.is_none()
        })
        .await;

        let later_deadline = runtime.begin_deadline();
        assert_eq!(
            runtime
                .build_authorization_url(
                    &later_deadline,
                    OAuthProvider::Google,
                    "https://nebula.example/api/v1/auth/oauth/google/callback",
                    "later-state",
                    "later-challenge",
                )
                .await
                .expect_err("cooldown must fail without refetch"),
            OAuthFailureCode::DiscoveryUnavailable
        );
        assert_eq!(discovery.requests().len(), 1);
    }

    #[tokio::test]
    async fn discovery_failure_cooldown_suppresses_refetch_for_five_seconds() {
        let discovery =
            TlsFixture::spawn("oauth.test", |_request, _| TestResponse::failure(503)).await;
        let runtime = runtime_for_fixture(oidc_config(&discovery), &discovery);

        for _ in 0..2 {
            let deadline = runtime.begin_deadline();
            assert_eq!(
                runtime
                    .build_authorization_url(
                        &deadline,
                        OAuthProvider::Google,
                        "https://nebula.example/api/v1/auth/oauth/google/callback",
                        "state",
                        "challenge",
                    )
                    .await
                    .expect_err("failed discovery must remain unavailable"),
                OAuthFailureCode::DiscoveryUnavailable
            );
        }
        assert_eq!(
            discovery.requests().len(),
            1,
            "cooldown must suppress retry"
        );

        let slot = Arc::clone(
            runtime
                .discovery
                .get(&OAuthProvider::Google)
                .expect("provider cooldown slot must exist")
                .value(),
        );
        let mut state = slot.lock().await;
        let retry_at = state
            .retry_not_before
            .expect("failed discovery must install cooldown");
        let remaining = retry_at.saturating_duration_since(Instant::now());
        assert!(remaining <= DISCOVERY_FAILURE_COOLDOWN);
        assert!(remaining > Duration::from_secs(4));
        state.retry_not_before = Some(Instant::now() - Duration::from_millis(1));
        drop(state);

        let deadline = runtime.begin_deadline();
        let _ = runtime
            .build_authorization_url(
                &deadline,
                OAuthProvider::Google,
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "state-after-cooldown",
                "challenge-after-cooldown",
            )
            .await;
        assert_eq!(
            discovery.requests().len(),
            2,
            "elapsed cooldown permits retry"
        );
    }

    #[tokio::test]
    async fn verified_email_fallback_reuses_the_original_absolute_deadline() {
        let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
            "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#)
                .delayed(Duration::from_millis(80)),
            "/userinfo" => TestResponse::json(r#"{"id":44}"#).delayed(Duration::from_millis(80)),
            "/emails" => TestResponse::json(
                r#"[{"email":"verified@example.com","primary":true,"verified":true}]"#,
            )
            .delayed(Duration::from_millis(150)),
            _ => TestResponse::failure(404),
        })
        .await;
        let runtime = runtime_for_fixture(manual_config(&fixture), &fixture);
        let deadline = OAuthFlowDeadline {
            expires_at: Instant::now() + Duration::from_millis(250),
        };
        let original_expiry = deadline.expires_at;
        let pending = runtime
            .begin_identity_completion(
                deadline,
                OAuthProvider::GitHub,
                "state",
                "code",
                "https://nebula.example/api/v1/auth/oauth/github/callback",
                "verifier",
            )
            .await
            .expect("primary identity stages fit within the shared budget");
        assert_eq!(pending.deadline.expires_at, original_expiry);
        let error = match runtime.resolve_verified_identity(pending).await {
            Ok(_) => panic!("fallback must not receive a fresh deadline"),
            Err(error) => error,
        };
        assert_eq!(error, OAuthFailureCode::CompletionTimeout);
        fixture.wait_for_request_count(3).await;
    }

    fn encoded_test_id_token(header_alg: &str, claims: serde_json::Value) -> SecretString {
        let header = serde_json::json!({"alg": header_alg, "typ": "JWT"});
        let compact = format!(
            "{}.{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("serialize JWT header")),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("serialize JWT claims")),
            URL_SAFE_NO_PAD.encode(b"syntactic-test-signature")
        );
        SecretString::new(compact.into_boxed_str())
    }

    fn valid_google_claims(
        now: i64,
        client_id: &str,
        state: &str,
        access_token: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "iss": GOOGLE_ISSUER,
            "sub": "google-subject-123",
            "aud": client_id,
            "exp": now + 600,
            "iat": now - 10,
            "nonce": nonce_for_state(state),
            "at_hash": expected_oidc_at_hash(access_token),
        })
    }

    #[test]
    fn oidc_at_hash_matches_the_core_rs256_vector() {
        assert_eq!(
            expected_oidc_at_hash("jHkWEdUXMU1BwAsC4vtUsZwnNvTIxEl0z9K3vx5KF0Y"),
            "77QmUPtjPfzWtF2AnpK9RQ"
        );
    }

    #[test]
    fn google_id_token_validation_rejects_every_identity_binding_mismatch() {
        const CLIENT_ID: &str = "google-client-id";
        const STATE: &str = "state-with-enough-randomness-for-test";
        const ACCESS_TOKEN: &str = "google-access-token";
        let now = 1_800_000_000;
        let client_id = SecretString::new(CLIENT_ID.to_owned().into_boxed_str());
        let access_token = SecretString::new(ACCESS_TOKEN.to_owned().into_boxed_str());

        let valid = encoded_test_id_token(
            "RS256",
            valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN),
        );
        let subject = validate_google_id_token(
            &valid,
            &access_token,
            &client_id,
            &nonce_for_state(STATE),
            now,
        )
        .expect("complete direct-token-endpoint evidence is valid");
        assert_eq!(subject.as_str(), "google-subject-123");

        let mut invalid = Vec::new();
        invalid.push(encoded_test_id_token(
            "none",
            valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN),
        ));
        for (field, value) in [
            ("iss", serde_json::json!("https://issuer.example.test")),
            ("aud", serde_json::json!("other-client")),
            ("exp", serde_json::json!(now - 61)),
            ("iat", serde_json::json!(now + 61)),
            ("nonce", serde_json::json!("wrong-nonce")),
            ("at_hash", serde_json::json!("wrong-at-hash")),
        ] {
            let mut claims = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
            claims[field] = value;
            invalid.push(encoded_test_id_token("RS256", claims));
        }
        let mut missing_at_hash = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
        missing_at_hash
            .as_object_mut()
            .expect("claims are an object")
            .remove("at_hash");
        invalid.push(encoded_test_id_token("RS256", missing_at_hash));
        let mut multi_audience = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
        multi_audience["aud"] = serde_json::json!([CLIENT_ID, CLIENT_ID]);
        invalid.push(encoded_test_id_token("RS256", multi_audience));
        let mut wrong_azp = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
        wrong_azp["azp"] = serde_json::json!("other-client");
        invalid.push(encoded_test_id_token("RS256", wrong_azp));
        let mut co_audience = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
        co_audience["aud"] = serde_json::json!([CLIENT_ID, "other-client"]);
        co_audience["azp"] = serde_json::json!(CLIENT_ID);
        invalid.push(encoded_test_id_token("RS256", co_audience));

        for id_token in invalid {
            let error = match validate_google_id_token(
                &id_token,
                &access_token,
                &client_id,
                &nonce_for_state(STATE),
                now,
            ) {
                Ok(_) => panic!("mismatched ID-token evidence must fail closed"),
                Err(error) => error,
            };
            assert_eq!(error, OAuthFailureCode::ProviderResponseInvalid);
        }
    }

    #[test]
    fn google_signup_email_requires_gmail_or_matching_hosted_domain() {
        let gmail = validate_google_email(GoogleEmailEvidence {
            email: Some("User@Gmail.COM".to_owned()),
            email_verified: Some(true),
            hosted_domain: None,
        })
        .expect("verified Gmail is provisionable");
        assert_eq!(gmail.into_string(), "user@gmail.com");

        let workspace = validate_google_email(GoogleEmailEvidence {
            email: Some("User@Example.COM".to_owned()),
            email_verified: Some(true),
            hosted_domain: Some("example.com".to_owned()),
        })
        .expect("matching hosted domain is provisionable");
        assert_eq!(workspace.into_string(), "user@example.com");

        for evidence in [
            GoogleEmailEvidence {
                email: Some("user@example.com".to_owned()),
                email_verified: Some(true),
                hosted_domain: None,
            },
            GoogleEmailEvidence {
                email: Some("user@example.com".to_owned()),
                email_verified: Some(true),
                hosted_domain: Some("other.example".to_owned()),
            },
            GoogleEmailEvidence {
                email: Some("user@gmail.com".to_owned()),
                email_verified: Some(false),
                hosted_domain: None,
            },
        ] {
            let error = match validate_google_email(evidence) {
                Ok(_) => panic!("ineligible Google email evidence must be rejected"),
                Err(error) => error,
            };
            assert_eq!(error, OAuthFailureCode::VerifiedEmailUnavailable);
        }
    }

    #[tokio::test]
    async fn google_missing_or_mismatched_at_hash_stops_before_userinfo() {
        const STATE: &str = "google-state-for-at-hash-test";
        const ACCESS_TOKEN: &str = "google-access-token";

        let mut missing_claims =
            valid_google_claims(unix_timestamp(), "google-client-id", STATE, ACCESS_TOKEN);
        missing_claims
            .as_object_mut()
            .expect("claims are an object")
            .remove("at_hash");
        let missing_id_token = encoded_test_id_token("RS256", missing_claims);
        let missing_token_body = serde_json::json!({
            "access_token": ACCESS_TOKEN,
            "token_type": "Bearer",
            "id_token": missing_id_token.expose_secret(),
        })
        .to_string();
        let missing = TlsFixture::spawn("oauth.test", move |request, _| {
            match request.path.as_str() {
                "/token" => TestResponse::json(missing_token_body.clone()),
                "/userinfo" => TestResponse::json(r#"{"sub":"must-not-be-fetched"}"#),
                _ => TestResponse::failure(404),
            }
        })
        .await;
        let runtime = runtime_for_fixture(google_manual_test_config(&missing), &missing);
        let result = runtime
            .begin_identity_completion(
                runtime.begin_deadline(),
                OAuthProvider::Google,
                STATE,
                "code",
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "verifier",
            )
            .await;
        assert!(matches!(
            result,
            Err(OAuthFailureCode::ProviderResponseInvalid)
        ));
        assert_eq!(
            missing
                .requests()
                .into_iter()
                .map(|request| request.path)
                .collect::<Vec<_>>(),
            ["/token"]
        );

        let mut claims =
            valid_google_claims(unix_timestamp(), "google-client-id", STATE, ACCESS_TOKEN);
        claims["at_hash"] = serde_json::json!("mismatched-at-hash");
        let id_token = encoded_test_id_token("RS256", claims);
        let token_body = serde_json::json!({
            "access_token": ACCESS_TOKEN,
            "token_type": "Bearer",
            "id_token": id_token.expose_secret(),
        })
        .to_string();
        let mismatch = TlsFixture::spawn("oauth.test", move |request, _| {
            match request.path.as_str() {
                "/token" => TestResponse::json(token_body.clone()),
                "/userinfo" => TestResponse::json(r#"{"sub":"must-not-be-fetched"}"#),
                _ => TestResponse::failure(404),
            }
        })
        .await;
        let runtime = runtime_for_fixture(google_manual_test_config(&mismatch), &mismatch);
        let result = runtime
            .begin_identity_completion(
                runtime.begin_deadline(),
                OAuthProvider::Google,
                STATE,
                "code",
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "verifier",
            )
            .await;
        assert!(matches!(
            result,
            Err(OAuthFailureCode::ProviderResponseInvalid)
        ));
        assert_eq!(
            mismatch
                .requests()
                .into_iter()
                .map(|request| request.path)
                .collect::<Vec<_>>(),
            ["/token"]
        );
    }

    #[test]
    fn pending_identity_and_wire_types_have_no_debug_surface() {
        static_assertions::assert_not_impl_any!(PendingExternalIdentity: std::fmt::Debug, Clone);
        static_assertions::assert_not_impl_any!(VerifiedEmailCapability: std::fmt::Debug, Clone);
        static_assertions::assert_not_impl_any!(TokenWireResponse: std::fmt::Debug);
        static_assertions::assert_not_impl_any!(GoogleUserinfoWire: std::fmt::Debug);
        static_assertions::assert_not_impl_any!(GitHubUserinfoWire: std::fmt::Debug);
        static_assertions::assert_not_impl_any!(ProvisionableEmail: std::fmt::Debug, Clone);
    }

    #[test]
    fn provider_identity_fields_are_normalized_and_bounded() {
        assert_eq!(
            normalize_verified_email(" User@Example.COM ".to_owned())
                .expect("valid email normalizes"),
            "user@example.com"
        );
        for email in [
            "   ".to_owned(),
            "missing-at".to_owned(),
            "a@b@c".to_owned(),
            "a b@example.com".to_owned(),
            "a@-example.com".to_owned(),
            "a@example..com".to_owned(),
            "x".repeat(255),
        ] {
            assert!(normalize_verified_email(email).is_err());
        }
        for subject in ["", " leading", "trailing ", "line\nbreak"] {
            assert!(validate_subject(subject).is_err());
        }
        assert!(validate_subject(&"s".repeat(256)).is_err());
        assert!(validate_subject(&"s".repeat(255)).is_ok());
    }

    #[test]
    fn token_wire_requires_non_empty_bearer_token() {
        for invalid in [
            r#"{"access_token":"","token_type":"Bearer"}"#,
            r#"{"access_token":"secret","token_type":"MAC"}"#,
            r#"{"access_token":"secret"}"#,
        ] {
            let parsed = serde_json::from_str::<TokenWireResponse>(invalid);
            assert!(
                parsed.is_err()
                    || parsed.is_ok_and(|token| {
                        token.access_token.expose_secret().is_empty()
                            || !token.token_type.eq_ignore_ascii_case("bearer")
                    })
            );
        }
        let token: TokenWireResponse =
            serde_json::from_str(r#"{"access_token":"secret","token_type":"bEaReR"}"#)
                .expect("case-insensitive bearer is valid");
        assert_eq!(token.token_type, "bEaReR");

        for invalid in [
            " leading",
            "trailing ",
            "embedded space",
            "line\nbreak",
            "tab\tbreak",
            "",
        ] {
            assert!(!valid_access_token(invalid), "accepted token {invalid:?}");
        }
        assert!(!valid_access_token(&"x".repeat(16 * 1024 + 1)));
        assert!(valid_access_token(&"x".repeat(16 * 1024)));
    }

    #[test]
    fn oidc_token_auth_method_selection_is_normative_and_closed() {
        assert_eq!(
            select_discovered_token_auth_method(None).expect("OIDC omitted-field default"),
            TokenEndpointAuthMethod::ClientSecretBasic
        );
        assert_eq!(
            select_discovered_token_auth_method(Some(&[
                "client_secret_post".to_owned(),
                "client_secret_basic".to_owned(),
            ]))
            .expect("Basic is preferred when both are advertised"),
            TokenEndpointAuthMethod::ClientSecretBasic
        );
        assert_eq!(
            select_discovered_token_auth_method(Some(&["client_secret_post".to_owned()]))
                .expect("post is supported when Basic is absent"),
            TokenEndpointAuthMethod::ClientSecretPost
        );
        assert_eq!(
            select_discovered_token_auth_method(Some(&["private_key_jwt".to_owned()]))
                .expect_err("unsupported-only discovery list must fail closed"),
            OAuthFailureCode::DiscoveryUnavailable
        );
    }
}
