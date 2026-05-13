//! `VaultProvider` — HashiCorp Vault implementation of the
//! [`ExternalProvider`] / [`LeasedProvider`] trait surface.
//!
//! See the crate-level docs for the path-shape convention used to switch
//! between KV v2 static reads and dynamic-secret reads.

use std::{borrow::Cow, time::Duration};

use chrono::Utc;
use nebula_credential::{
    SecretString,
    provider::{
        ExternalProvider, ExternalReference, LeaseHandle, LeasedProvider, ProviderError,
        ProviderFuture, ProviderResolution,
    },
};
use reqwest::{Client, Response, StatusCode};
use serde_json::{Map, Value};
use tracing::Instrument as _;
use url::Url;

use crate::wire::{DynamicSecretEnvelope, KvV2Envelope, LeaseRenewEnvelope};

/// Provider attribution stamped onto every [`LeaseHandle`] this backend issues.
///
/// Matches [`ExternalProvider::provider_name`] so the default
/// [`LeasedProvider::handles_lease`] routes correctly through composed
/// providers (chain / cache layer).
const PROVIDER_NAME: &str = "vault";

/// Prefix on [`ExternalReference::path`] that routes the resolution to the
/// dynamic-secret read path (`GET /v1/<rest>`). Anything else is treated as
/// a KV v2 path under the configured mount.
const DYNAMIC_PATH_PREFIX: &str = "dyn/";

/// Header carrying the Vault token on every request.
const TOKEN_HEADER: &str = "X-Vault-Token";

/// Construction-time errors for [`VaultProvider`].
///
/// These are surfaced when wiring the backend (typically at process start),
/// not on the hot resolve path — separate from [`ProviderError`] so a
/// composition root can react synchronously.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultError {
    /// The supplied address could not be parsed as an absolute URL or had a
    /// non-HTTP(S) scheme.
    #[error("vault address must be an absolute http(s) URL: {reason}")]
    InvalidAddress {
        /// Human-readable cause.
        reason: String,
    },
    /// `reqwest::Client::build` failed (TLS misconfiguration, runtime issue).
    #[error("failed to build reqwest client: {reason}")]
    ClientBuild {
        /// Human-readable cause.
        reason: String,
    },
}

/// Builder-style configuration for [`VaultProvider`].
///
/// `kv_mount` is the canonical KV v2 mount path Vault projects ship with
/// (`secret/`) — keep it without a leading slash; the provider joins it into
/// the URL itself.
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Vault server address — e.g. `https://vault.example.com:8200/`.
    /// Trailing slash is permitted; the provider normalises away the
    /// difference.
    pub address: Url,
    /// Vault authentication token. Treated as a secret — never logged.
    pub token: SecretString,
    /// KV v2 mount path. `"secret"` is the Vault default for `vault kv` CLI.
    pub kv_mount: String,
    /// Per-request timeout for the underlying HTTP client.
    pub request_timeout: Duration,
}

impl VaultConfig {
    /// Build a config with the workspace defaults (`kv_mount = "secret"`,
    /// `request_timeout = 10s`).
    #[must_use]
    pub fn new(address: Url, token: SecretString) -> Self {
        Self {
            address,
            token,
            kv_mount: "secret".to_owned(),
            request_timeout: Duration::from_secs(10),
        }
    }
}

/// HashiCorp Vault backend implementing
/// [`ExternalProvider`] + [`LeasedProvider`].
///
/// See the crate-level docs for the path-shape convention and the request
/// semantics. Errors classify per ADR-0051:
///
/// - 404 → [`ProviderError::NotFound`] (chain fall-through).
/// - 403 → [`ProviderError::AccessDenied`] (short-circuit).
/// - Network / timeout / 5xx → [`ProviderError::Unavailable`] (short-circuit).
/// - Other 4xx, decode failure → [`ProviderError::Backend`] (short-circuit).
pub struct VaultProvider {
    address: Url,
    token: SecretString,
    kv_mount: String,
    client: Client,
}

impl std::fmt::Debug for VaultProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Do NOT print the token — it is a bearer credential against the
        // Vault server. The address is safe to surface.
        f.debug_struct("VaultProvider")
            .field("address", &self.address.as_str())
            .field("kv_mount", &self.kv_mount)
            .finish_non_exhaustive()
    }
}

impl VaultProvider {
    /// Build a provider with a fresh `reqwest::Client`.
    ///
    /// Use [`Self::with_client`] when sharing an `HTTP/2` pool across
    /// providers, or when a host policy (proxy, root CA pinning) requires
    /// a pre-configured client.
    pub fn new(config: VaultConfig) -> Result<Self, VaultError> {
        validate_address(&config.address)?;
        let client = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|err| VaultError::ClientBuild {
                reason: err.to_string(),
            })?;
        Ok(Self::with_client(config, client))
    }

    /// Build a provider over a caller-supplied `reqwest::Client`. The
    /// timeout in `config.request_timeout` is ignored on this path — the
    /// caller's client owns that policy.
    #[must_use]
    pub fn with_client(config: VaultConfig, client: Client) -> Self {
        Self {
            address: config.address,
            token: config.token,
            kv_mount: config.kv_mount,
            client,
        }
    }

    /// `<address>/v1/<segments>` with normalised slashes.
    fn api_url(&self, segments: &str) -> Result<Url, ProviderError> {
        // `Url::join` is fussy about base trailing slashes; rebuild from
        // the address path to avoid surprises like a missing slash dropping
        // the last segment of the base.
        let mut base = self.address.clone();
        let base_path = base.path().trim_end_matches('/').to_owned();
        let trimmed = segments.trim_start_matches('/');
        base.set_path(&format!("{base_path}/v1/{trimmed}"));
        Ok(base)
    }

    /// KV v2 read URL with optional `?version=` query.
    fn kv_url(&self, secret_path: &str, version: Option<&str>) -> Result<Url, ProviderError> {
        let secret_path = secret_path.trim_start_matches('/');
        let mount = self.kv_mount.trim_matches('/');
        let mut url = self.api_url(&format!("{mount}/data/{secret_path}"))?;
        if let Some(v) = version {
            url.query_pairs_mut().append_pair("version", v);
        }
        Ok(url)
    }

    /// Apply common request headers (token) and execute the request,
    /// classifying transport-layer failures into [`ProviderError`].
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Response, ProviderError> {
        request
            .header(TOKEN_HEADER, self.token.expose_secret())
            .send()
            .await
            .map_err(classify_transport_error)
    }

    async fn do_resolve(
        &self,
        reference: &ExternalReference,
    ) -> Result<ProviderResolution, ProviderError> {
        if let Some(rest) = reference.path.strip_prefix(DYNAMIC_PATH_PREFIX) {
            let span = tracing::debug_span!(
                "vault_resolve",
                kind = "dynamic",
                path = %reference.path,
            );
            self.resolve_dynamic(rest, reference.field.as_deref())
                .instrument(span)
                .await
        } else {
            let span = tracing::debug_span!(
                "vault_resolve",
                kind = "kv2",
                path = %reference.path,
                mount = %self.kv_mount,
            );
            self.resolve_kv_v2(
                &reference.path,
                reference.version.as_deref(),
                reference.field.as_deref(),
            )
            .instrument(span)
            .await
        }
    }

    async fn resolve_kv_v2(
        &self,
        secret_path: &str,
        version: Option<&str>,
        field: Option<&str>,
    ) -> Result<ProviderResolution, ProviderError> {
        let url = self.kv_url(secret_path, version)?;
        tracing::debug!(target: "nebula_credential_vault", url = %url, "vault kv2 GET");
        let response = self.send(self.client.get(url)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(classify_status(status, secret_path));
        }
        let envelope: KvV2Envelope = response.json().await.map_err(|err| {
            ProviderError::Backend(format!("vault kv2 body decode: {err}").into())
        })?;
        let secret = extract_secret(envelope.data.data, field)?;
        Ok(ProviderResolution::from_secret(SecretString::new(secret)))
    }

    async fn resolve_dynamic(
        &self,
        rest_path: &str,
        field: Option<&str>,
    ) -> Result<ProviderResolution, ProviderError> {
        let url = self.api_url(rest_path)?;
        tracing::debug!(target: "nebula_credential_vault", url = %url, "vault dynamic GET");
        let response = self.send(self.client.get(url)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(classify_status(status, rest_path));
        }
        let envelope: DynamicSecretEnvelope = response.json().await.map_err(|err| {
            ProviderError::Backend(format!("vault dynamic body decode: {err}").into())
        })?;
        let lease = LeaseHandle::new(
            Cow::Borrowed(PROVIDER_NAME),
            envelope.lease_id,
            Utc::now(),
            Duration::from_secs(envelope.lease_duration),
        );
        let secret = extract_secret(envelope.data, field)?;
        Ok(ProviderResolution::with_lease(
            SecretString::new(secret),
            lease,
        ))
    }

    async fn do_renew(&self, lease: &LeaseHandle) -> Result<ProviderResolution, ProviderError> {
        let url = self.api_url("sys/leases/renew")?;
        tracing::debug!(
            target: "nebula_credential_vault",
            url = %url,
            lease_id = %lease.lease_id,
            "vault renew",
        );
        let body = serde_json::json!({
            "lease_id": lease.lease_id,
            "increment": lease.ttl.as_secs(),
        });
        let response = self.send(self.client.put(url).json(&body)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(classify_status(status, &lease.lease_id));
        }
        let envelope: LeaseRenewEnvelope = response.json().await.map_err(|err| {
            ProviderError::Backend(format!("vault renew body decode: {err}").into())
        })?;
        // `/sys/leases/renew` returns lease metadata only — the original
        // secret payload is not echoed. The resolution is metadata-only;
        // callers refresh from the cache layer or re-resolve to pick up
        // any rolled credential.
        let lease = LeaseHandle::new(
            Cow::Borrowed(PROVIDER_NAME),
            envelope.lease_id,
            Utc::now(),
            Duration::from_secs(envelope.lease_duration),
        );
        Ok(ProviderResolution::with_lease(
            SecretString::new(String::new()),
            lease,
        ))
    }

    async fn do_revoke(&self, lease: &LeaseHandle) -> Result<ProviderResolution, ProviderError> {
        let url = self.api_url("sys/leases/revoke")?;
        tracing::debug!(
            target: "nebula_credential_vault",
            url = %url,
            lease_id = %lease.lease_id,
            "vault revoke",
        );
        let body = serde_json::json!({ "lease_id": lease.lease_id });
        let response = self.send(self.client.put(url).json(&body)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(classify_status(status, &lease.lease_id));
        }
        Ok(ProviderResolution::empty())
    }
}

impl ExternalProvider for VaultProvider {
    fn resolve<'a>(&'a self, reference: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::new(async move { self.do_resolve(reference).await })
    }

    fn provider_name(&self) -> &str {
        PROVIDER_NAME
    }

    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        Some(self)
    }
}

impl LeasedProvider for VaultProvider {
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        ProviderFuture::new(async move {
            // Defensive routing: the chain / cache layer call `handles_lease`
            // before dispatching, but a hand-built dispatcher might not —
            // refuse to act on a lease attributed to a different backend.
            if !self.handles_lease(lease) {
                tracing::warn!(
                    target: "nebula_credential_vault",
                    lease_id = %lease.lease_id,
                    lease_provider = %lease.provider,
                    "renew rejected: lease not attributed to this provider",
                );
                return Err(ProviderError::NotFound {
                    path: format!(
                        "lease {} attributed to {}, not {}",
                        lease.lease_id, lease.provider, PROVIDER_NAME
                    ),
                });
            }
            self.do_renew(lease).await
        })
    }

    fn revoke<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        ProviderFuture::new(async move {
            if !self.handles_lease(lease) {
                tracing::warn!(
                    target: "nebula_credential_vault",
                    lease_id = %lease.lease_id,
                    lease_provider = %lease.provider,
                    "revoke rejected: lease not attributed to this provider",
                );
                return Err(ProviderError::NotFound {
                    path: format!(
                        "lease {} attributed to {}, not {}",
                        lease.lease_id, lease.provider, PROVIDER_NAME
                    ),
                });
            }
            self.do_revoke(lease).await
        })
    }
}

fn validate_address(address: &Url) -> Result<(), VaultError> {
    match address.scheme() {
        "http" | "https" => Ok(()),
        other => Err(VaultError::InvalidAddress {
            reason: format!("unsupported scheme: {other}"),
        }),
    }
}

/// Map an HTTP status into a [`ProviderError`].
///
/// 4xx codes other than 404/403 fall into [`ProviderError::Backend`] because
/// they typically signal malformed requests (sealed Vault, wrong API
/// version) that no fall-through retry would heal — short-circuiting the
/// chain there is the safer default.
fn classify_status(status: StatusCode, context: &str) -> ProviderError {
    match status {
        StatusCode::NOT_FOUND => ProviderError::NotFound {
            path: context.to_owned(),
        },
        StatusCode::FORBIDDEN => ProviderError::AccessDenied {
            reason: format!("vault rejected token for {context}"),
        },
        s if s.is_server_error() => {
            tracing::warn!(
                target: "nebula_credential_vault",
                status = %s,
                context = %context,
                "vault server error",
            );
            ProviderError::Unavailable {
                reason: format!("vault status {s}"),
            }
        },
        s => {
            tracing::warn!(
                target: "nebula_credential_vault",
                status = %s,
                context = %context,
                "vault client error",
            );
            ProviderError::Backend(format!("vault unexpected status {s}").into())
        },
    }
}

/// Map a `reqwest` transport error into a [`ProviderError`].
///
/// Connect / timeout / decode-at-transport all classify as [`Unavailable`]
/// — the request never reached a meaningful status. Treating these as
/// `Backend` would mask retryable network issues behind the chain's
/// short-circuit; treating them as `NotFound` would mask connectivity
/// loss as "not configured", which is worse.
fn classify_transport_error(err: reqwest::Error) -> ProviderError {
    ProviderError::Unavailable {
        reason: format!("vault transport: {err}"),
    }
}

/// Extract the requested field from a Vault `data` map, encoding the full
/// payload when no field is requested.
///
/// Strings are returned verbatim (so a `password` field containing
/// `"hunter2"` decodes cleanly). Non-string scalars / objects are
/// JSON-encoded — that matches what `vault kv get -format=json` emits and
/// lets callers parse structured payloads downstream.
fn extract_secret(
    mut data: std::collections::BTreeMap<String, Value>,
    field: Option<&str>,
) -> Result<String, ProviderError> {
    if let Some(name) = field {
        let Some(value) = data.remove(name) else {
            return Err(ProviderError::NotFound {
                path: format!("field {name}"),
            });
        };
        return Ok(match value {
            Value::String(s) => s,
            other => other.to_string(),
        });
    }
    // `BTreeMap<String, Value>` serialises as a JSON object with
    // sorted keys — deterministic output is useful for caching /
    // diffing the resolved blob.
    let map: Map<String, Value> = data.into_iter().collect();
    serde_json::to_string(&Value::Object(map))
        .map_err(|err| ProviderError::Backend(format!("vault data encode: {err}").into()))
}

#[cfg(test)]
mod tests {
    use nebula_credential::provider::ProviderKind;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path, query_param},
    };

    use super::*;

    // ────────────────────────────────────────────────────────────────────
    // Helpers
    // ────────────────────────────────────────────────────────────────────

    /// Build a `VaultProvider` pointed at `server` with a fixed token.
    fn vault(server: &MockServer) -> VaultProvider {
        let url = Url::parse(&server.uri()).expect("mock server URL parses");
        let config = VaultConfig::new(url, SecretString::new("test-token"));
        VaultProvider::new(config).expect("provider builds")
    }

    fn kv_ref(p: &str, field: Option<&str>) -> ExternalReference {
        ExternalReference {
            provider: ProviderKind::Vault,
            path: p.to_owned(),
            version: None,
            field: field.map(str::to_owned),
        }
    }

    fn lease(provider: &'static str, id: &str) -> LeaseHandle {
        LeaseHandle::new(provider, id, Utc::now(), Duration::from_mins(1))
    }

    // ────────────────────────────────────────────────────────────────────
    // C2 — KV v2 static path.
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn kv_v2_happy_path_returns_requested_field() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/my-app/db"))
            .and(header(TOKEN_HEADER, "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "data": { "password": "hunter2", "username": "svc" },
                    "metadata": { "version": 1 }
                }
            })))
            .mount(&server)
            .await;

        let provider = vault(&server);
        let res = provider
            .resolve(&kv_ref("my-app/db", Some("password")))
            .await
            .expect("resolve succeeds");
        assert_eq!(res.secret.expose_secret(), "hunter2");
        assert!(res.lease.is_none(), "static KV resolutions carry no lease");
        assert!(res.ttl.is_none(), "static KV resolutions carry no TTL");
    }

    #[tokio::test]
    async fn kv_v2_version_appended_as_query_param() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/my-app/db"))
            .and(query_param("version", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "data": { "password": "v3" }, "metadata": {} }
            })))
            .mount(&server)
            .await;

        let provider = vault(&server);
        let mut reference = kv_ref("my-app/db", Some("password"));
        reference.version = Some("3".to_owned());
        let res = provider.resolve(&reference).await.expect("resolve ok");
        assert_eq!(res.secret.expose_secret(), "v3");
    }

    #[tokio::test]
    async fn kv_v2_404_maps_to_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let err = vault(&server)
            .resolve(&kv_ref("missing", None))
            .await
            .expect_err("404 must be NotFound");
        assert!(
            matches!(err, ProviderError::NotFound { .. }),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn kv_v2_403_maps_to_access_denied() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/forbidden"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let err = vault(&server)
            .resolve(&kv_ref("forbidden", None))
            .await
            .expect_err("403 must be AccessDenied");
        assert!(
            matches!(err, ProviderError::AccessDenied { .. }),
            "expected AccessDenied, got {err:?}"
        );
    }

    #[tokio::test]
    async fn kv_v2_5xx_maps_to_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/secret/data/down"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let err = vault(&server)
            .resolve(&kv_ref("down", None))
            .await
            .expect_err("5xx must be Unavailable");
        assert!(
            matches!(err, ProviderError::Unavailable { .. }),
            "expected Unavailable, got {err:?}"
        );
    }

    #[tokio::test]
    async fn transport_failure_maps_to_unavailable() {
        // Target a privileged port that no test process can bind: 127.0.0.1:1
        // refuses connections on every platform the workspace targets,
        // producing a `reqwest` transport error rather than a status code.
        // Classifying that as `Unavailable` keeps connectivity loss
        // distinguishable from "secret not configured" (`NotFound` would
        // mask the operational signal and trigger chain fall-through).
        let provider = VaultProvider::new(VaultConfig {
            address: Url::parse("http://127.0.0.1:1/").expect("URL parses"),
            token: SecretString::new("test-token"),
            kv_mount: "secret".to_owned(),
            // Tight timeout so the test fails fast on hosts that drop the
            // connect attempt silently (some VPNs / EDRs do).
            request_timeout: Duration::from_secs(2),
        })
        .expect("provider builds");

        let err = provider
            .resolve(&kv_ref("anything", None))
            .await
            .expect_err("closed port must surface as transport error");
        assert!(
            matches!(err, ProviderError::Unavailable { .. }),
            "expected Unavailable, got {err:?}"
        );
    }

    // ────────────────────────────────────────────────────────────────────
    // C3 — dynamic secrets path.
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn dynamic_path_returns_resolution_with_lease() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/database/creds/my-role"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "request_id": "abc",
                "lease_id": "database/creds/my-role/xyz",
                "lease_duration": 3600,
                "renewable": true,
                "data": { "username": "u-1", "password": "p-1" }
            })))
            .mount(&server)
            .await;

        let provider = vault(&server);
        let res = provider
            .resolve(&kv_ref("dyn/database/creds/my-role", Some("password")))
            .await
            .expect("dynamic resolve succeeds");
        assert_eq!(res.secret.expose_secret(), "p-1");
        let lease = res
            .lease
            .as_ref()
            .expect("dynamic resolutions carry a lease");
        assert_eq!(
            lease.provider, "vault",
            "lease attributed to the vault backend"
        );
        assert_eq!(lease.lease_id, "database/creds/my-role/xyz");
        assert_eq!(
            lease.ttl,
            Duration::from_hours(1),
            "TTL is the Vault-reported lease_duration"
        );
        // The convenience constructor copies the lease TTL into the
        // resolution-level `ttl` field so the cache layer sees it without
        // a lease.unwrap().
        assert_eq!(res.ttl, Some(Duration::from_hours(1)));
    }

    // ────────────────────────────────────────────────────────────────────
    // C4 — LeasedProvider lifecycle.
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn handles_lease_matches_provider_name() {
        // Defensive routing rule: a Vault provider must claim only leases
        // it issued. Attribution comes from `LeaseHandle::provider`, which
        // we stamp as "vault" at resolve time.
        let provider = VaultProvider::new(VaultConfig::new(
            Url::parse("http://127.0.0.1:1").expect("URL parses"),
            SecretString::new("token"),
        ))
        .expect("provider builds");
        assert!(
            provider.handles_lease(&lease("vault", "lease-1")),
            "lease attributed to vault is handled"
        );
        assert!(
            !provider.handles_lease(&lease("aws-sm", "lease-2")),
            "lease attributed to another backend is rejected"
        );
    }

    #[tokio::test]
    async fn renew_extends_lease_and_returns_metadata_only() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/v1/sys/leases/renew"))
            .and(body_json(json!({ "lease_id": "lease-1", "increment": 60 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lease_id": "lease-1",
                "lease_duration": 7200,
                "renewable": true
            })))
            .mount(&server)
            .await;

        let provider = vault(&server);
        let res = provider
            .renew(&lease("vault", "lease-1"))
            .await
            .expect("renew succeeds");
        let renewed = res.lease.as_ref().expect("renew returns lease metadata");
        assert_eq!(renewed.lease_id, "lease-1");
        assert_eq!(
            renewed.ttl,
            Duration::from_hours(2),
            "renew honours the server-reported new lease_duration"
        );
        // Vault's `/renew` doesn't echo the secret payload — the resolution
        // is intentionally metadata-only.
        assert!(
            res.secret.expose_secret().is_empty(),
            "renew resolution carries no secret material"
        );
    }

    #[tokio::test]
    async fn revoke_returns_empty_resolution() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/v1/sys/leases/revoke"))
            .and(body_json(json!({ "lease_id": "lease-1" })))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let provider = vault(&server);
        let res = provider
            .revoke(&lease("vault", "lease-1"))
            .await
            .expect("revoke succeeds");
        assert!(res.secret.expose_secret().is_empty());
        assert!(res.lease.is_none());
        assert!(res.ttl.is_none());
    }

    #[tokio::test]
    async fn renew_misroute_rejected_with_not_found() {
        // Per the Phase B contract: a hand-built dispatcher that routes a
        // foreign lease through this backend must NOT silently act on it.
        // No mock — the test asserts the provider rejects before any HTTP
        // call is made.
        let server = MockServer::start().await;
        // Register a catch-all that would respond 200 if reached; the test
        // proves it's never reached.
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lease_id": "foreign",
                "lease_duration": 10
            })))
            .mount(&server)
            .await;

        let provider = vault(&server);
        let err = provider
            .renew(&lease("aws-sm", "foreign"))
            .await
            .expect_err("misrouted renew must fail");
        assert!(
            matches!(err, ProviderError::NotFound { .. }),
            "expected NotFound for foreign attribution, got {err:?}"
        );
        // The mock recorded zero matching requests because the provider
        // returned before the HTTP layer.
        let requests = server.received_requests().await.expect("recorder");
        assert!(
            requests.is_empty(),
            "misrouted renew must short-circuit before any HTTP call"
        );
    }

    #[tokio::test]
    async fn revoke_misroute_rejected_with_not_found() {
        let server = MockServer::start().await;
        let provider = vault(&server);
        let err = provider
            .revoke(&lease("aws-sm", "foreign"))
            .await
            .expect_err("misrouted revoke must fail");
        assert!(matches!(err, ProviderError::NotFound { .. }));
        let requests = server.received_requests().await.expect("recorder");
        assert!(requests.is_empty());
    }

    // ────────────────────────────────────────────────────────────────────
    // Capability discovery + attribution sanity.
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn lease_renewal_surfaces_self() {
        let server = MockServer::start().await;
        let provider = vault(&server);
        let view = provider
            .lease_renewal()
            .expect("vault advertises lease capability");
        assert_eq!(view.provider_name(), "vault");
    }

    #[test]
    fn invalid_address_rejected_at_construction() {
        // The validator runs before any HTTP call so a bad config fails
        // synchronously — important for composition roots that want to
        // surface misconfiguration at startup rather than on the first
        // resolve.
        let err = VaultProvider::new(VaultConfig::new(
            Url::parse("ftp://vault.example.com/").expect("URL parses"),
            SecretString::new("token"),
        ))
        .expect_err("non-http schemes are rejected");
        assert!(
            matches!(err, VaultError::InvalidAddress { .. }),
            "expected InvalidAddress, got {err:?}"
        );
    }

    #[test]
    fn extract_secret_returns_whole_data_when_no_field() {
        let mut data = std::collections::BTreeMap::new();
        data.insert("a".to_owned(), Value::String("1".to_owned()));
        data.insert("b".to_owned(), Value::Number(serde_json::Number::from(2)));
        let s = extract_secret(data, None).expect("encode ok");
        // BTreeMap keeps keys sorted, so output is deterministic.
        assert_eq!(s, r#"{"a":"1","b":2}"#);
    }

    #[test]
    fn extract_secret_missing_field_is_not_found() {
        let data = std::collections::BTreeMap::new();
        let err = extract_secret(data, Some("absent")).expect_err("missing field surfaces");
        assert!(matches!(err, ProviderError::NotFound { .. }));
    }

    #[test]
    fn provider_debug_does_not_leak_token() {
        let provider = VaultProvider::new(VaultConfig::new(
            Url::parse("https://vault.example.com/").expect("URL parses"),
            SecretString::new("super-secret-token"),
        ))
        .expect("provider builds");
        let rendered = format!("{provider:?}");
        assert!(
            !rendered.contains("super-secret-token"),
            "Debug must redact the auth token"
        );
    }
}
