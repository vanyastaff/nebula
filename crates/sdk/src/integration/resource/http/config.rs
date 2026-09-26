//! Operator configuration of an HTTP resource and the auth-neutral transport
//! built from it.

use std::{
    fmt,
    hash::{DefaultHasher, Hash, Hasher},
    net::IpAddr,
    sync::Arc,
    time::Duration,
};

use nebula_resource::{Error, ResourceConfig};
use reqwest::{Url, header::HeaderValue};
use serde::Deserialize;

const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_READ_IDLE_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const DEFAULT_MAX_STREAM_BYTES: u64 = 64 * 1024 * 1024;

/// Operator configuration of an HTTP resource: where the API lives and the
/// transport's bounds. It carries no credential; those come from the
/// resource's credential slots.
///
/// `base_url` is an origin plus an optional mount prefix
/// (`https://api.example.com/v3`); every request path is appended to it and
/// may not leave its origin. It must be `https`, or `http` for a loopback
/// host only, without userinfo, query or fragment.
///
/// Its `Debug` output redacts `base_url`.
#[derive(Clone, PartialEq, Eq, Hash, Deserialize, nebula_schema::Schema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HttpConfig {
    /// Origin plus optional mount prefix every request path is appended to.
    #[field(label = "Base URL")]
    pub base_url: String,
    /// Time allowed to establish a connection, TLS included, in milliseconds.
    #[field(label = "Connect timeout (ms)", default = 5000)]
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    /// Time allowed for one buffered request, response body included, in
    /// milliseconds. The unit's deadline may shorten it.
    #[field(label = "Request timeout (ms)", default = 30000)]
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    /// Longest silence allowed between two reads of a response, in
    /// milliseconds; it bounds a stalled stream.
    #[field(label = "Read idle timeout (ms)", default = 30000)]
    #[serde(default = "default_read_idle_timeout_ms")]
    pub read_idle_timeout_ms: u64,
    /// Largest buffered response body, in bytes.
    #[field(label = "Max response bytes", default = 1048576)]
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: u64,
    /// Largest streamed response body, in bytes.
    #[field(label = "Max stream bytes", default = 67108864)]
    #[serde(default = "default_max_stream_bytes")]
    pub max_stream_bytes: u64,
    /// PEM certificates trusted in addition to the platform's roots, for a
    /// private certificate authority.
    #[field(label = "Extra root certificates (PEM)")]
    #[serde(default)]
    pub extra_root_certificates_pem: Vec<String>,
    /// `User-Agent` sent with every request; the HTTP client's own when
    /// absent.
    #[field(label = "User agent")]
    #[serde(default)]
    pub user_agent: Option<String>,
}

fn default_connect_timeout_ms() -> u64 {
    DEFAULT_CONNECT_TIMEOUT_MS
}

fn default_request_timeout_ms() -> u64 {
    DEFAULT_REQUEST_TIMEOUT_MS
}

fn default_read_idle_timeout_ms() -> u64 {
    DEFAULT_READ_IDLE_TIMEOUT_MS
}

fn default_max_response_bytes() -> u64 {
    DEFAULT_MAX_RESPONSE_BYTES
}

fn default_max_stream_bytes() -> u64 {
    DEFAULT_MAX_STREAM_BYTES
}

impl HttpConfig {
    /// A configuration for `base_url` with the default bounds: 5 s connect,
    /// 30 s request, 30 s read idle, 1 MiB buffered and 64 MiB streamed
    /// response bodies, platform roots only.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            connect_timeout_ms: DEFAULT_CONNECT_TIMEOUT_MS,
            request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
            read_idle_timeout_ms: DEFAULT_READ_IDLE_TIMEOUT_MS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_stream_bytes: DEFAULT_MAX_STREAM_BYTES,
            extra_root_certificates_pem: Vec::new(),
            user_agent: None,
        }
    }

    /// Sets the connect timeout.
    #[must_use]
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout_ms = millis(timeout);
        self
    }

    /// Sets the buffered request timeout.
    #[must_use]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout_ms = millis(timeout);
        self
    }

    /// Sets the read idle timeout.
    #[must_use]
    pub fn with_read_idle_timeout(mut self, timeout: Duration) -> Self {
        self.read_idle_timeout_ms = millis(timeout);
        self
    }

    /// Sets the largest buffered response body.
    #[must_use]
    pub fn with_max_response_bytes(mut self, bytes: u64) -> Self {
        self.max_response_bytes = bytes;
        self
    }

    /// Sets the largest streamed response body.
    #[must_use]
    pub fn with_max_stream_bytes(mut self, bytes: u64) -> Self {
        self.max_stream_bytes = bytes;
        self
    }

    /// Trusts one more PEM root certificate (or bundle).
    #[must_use]
    pub fn with_extra_root_certificate_pem(mut self, pem: impl Into<String>) -> Self {
        self.extra_root_certificates_pem.push(pem.into());
        self
    }

    /// Sets the `User-Agent` header.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// The parsed base URL, checked as the type docs say.
    fn base(&self) -> Result<Url, Error> {
        let base = Url::parse(&self.base_url)
            .map_err(|_| Error::permanent("http base_url is not a valid absolute URL"))?;
        if base.cannot_be_a_base() || base.host_str().is_none() {
            return Err(Error::permanent("http base_url has no host"));
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(Error::permanent(
                "http base_url must not carry userinfo; credentials come from credential slots",
            ));
        }
        if base.query().is_some() || base.fragment().is_some() {
            return Err(Error::permanent(
                "http base_url must not carry a query or a fragment",
            ));
        }
        match base.scheme() {
            "https" => Ok(base),
            "http" if is_loopback(&base) => Ok(base),
            "http" => Err(Error::permanent(
                "http base_url must use https unless its host is loopback",
            )),
            _ => Err(Error::permanent("http base_url must use https")),
        }
    }

    /// The extra roots, each PEM parsed into at least one certificate.
    fn extra_roots(&self) -> Result<Vec<reqwest::Certificate>, Error> {
        let mut roots = Vec::new();
        for pem in &self.extra_root_certificates_pem {
            let certificates = reqwest::Certificate::from_pem_bundle(pem.as_bytes())
                .map_err(|_| Error::permanent("an extra root certificate is not valid PEM"))?;
            if certificates.is_empty() {
                return Err(Error::permanent(
                    "an extra root certificate holds no certificate",
                ));
            }
            roots.extend(certificates);
        }
        Ok(roots)
    }

    fn user_agent_header(&self) -> Result<Option<HeaderValue>, Error> {
        self.user_agent
            .as_deref()
            .map(|agent| {
                HeaderValue::from_str(agent)
                    .map_err(|_| Error::permanent("http user_agent is not a valid header value"))
            })
            .transpose()
    }
}

impl fmt::Debug for HttpConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpConfig")
            .field("base_url", &"<redacted>")
            .field("connect_timeout_ms", &self.connect_timeout_ms)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("read_idle_timeout_ms", &self.read_idle_timeout_ms)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_stream_bytes", &self.max_stream_bytes)
            .field(
                "extra_root_certificates",
                &self.extra_root_certificates_pem.len(),
            )
            .field("user_agent", &self.user_agent.is_some())
            .finish()
    }
}

impl ResourceConfig for HttpConfig {
    /// Rejects an unusable configuration with a permanent error whose text
    /// never echoes the URL.
    fn validate(&self) -> Result<(), Error> {
        self.base()?;
        for (value, what) in [
            (
                self.connect_timeout_ms,
                "http connect_timeout_ms must be positive",
            ),
            (
                self.request_timeout_ms,
                "http request_timeout_ms must be positive",
            ),
            (
                self.read_idle_timeout_ms,
                "http read_idle_timeout_ms must be positive",
            ),
            (
                self.max_response_bytes,
                "http max_response_bytes must be positive",
            ),
            (
                self.max_stream_bytes,
                "http max_stream_bytes must be positive",
            ),
        ] {
            if value == 0 {
                return Err(Error::permanent(what));
            }
        }
        self.extra_roots()?;
        self.user_agent_header()?;
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// The bounds a transport applies to every exchange.
#[derive(Clone, Copy)]
pub(super) struct Limits {
    /// Budget of one buffered exchange, body included.
    pub(super) request_timeout: Duration,
    /// Largest buffered response body.
    pub(super) max_response_bytes: u64,
    /// Largest streamed response body.
    pub(super) max_stream_bytes: u64,
}

struct TransportInner {
    client: reqwest::Client,
    base: Url,
    limits: Limits,
}

/// The auth-neutral HTTP transport of one resource instance: a connection
/// pool bound to one base URL, built once in `Provider::create` and reused
/// across credential rotations (credentials are applied per attempt, never
/// stored here).
///
/// Its policy is fixed: redirects are never followed (a `3xx` is an
/// answer; following it would be a new unit), requests are never retried by
/// the client, no proxy, no `Referer`, no cookie store, no default
/// credentials. TLS verifies against the platform's roots plus the
/// configured extra roots; there is no switch to skip verification. An
/// `https` base allows `https` only.
///
/// Cloning shares the pool. `Debug` shows no URL.
#[derive(Clone)]
pub struct HttpTransport {
    inner: Arc<TransportInner>,
}

impl HttpTransport {
    /// Builds the transport for `config`.
    ///
    /// # Errors
    ///
    /// A permanent [`Error`] when `config` does not validate or the client
    /// cannot be built; its text never echoes the URL.
    pub fn new(config: &HttpConfig) -> Result<Self, Error> {
        config.validate()?;
        let base = config.base()?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .referer(false)
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .read_timeout(Duration::from_millis(config.read_idle_timeout_ms))
            .https_only(base.scheme() == "https")
            .tls_certs_merge(config.extra_roots()?);
        if let Some(agent) = config.user_agent_header()? {
            builder = builder.user_agent(agent);
        }
        let client = builder
            .build()
            .map_err(|_| Error::permanent("the HTTP client could not be built"))?;
        Ok(Self {
            inner: Arc::new(TransportInner {
                client,
                base,
                limits: Limits {
                    request_timeout: Duration::from_millis(config.request_timeout_ms),
                    max_response_bytes: config.max_response_bytes,
                    max_stream_bytes: config.max_stream_bytes,
                },
            }),
        })
    }

    pub(super) fn client(&self) -> &reqwest::Client {
        &self.inner.client
    }

    pub(super) fn base(&self) -> &Url {
        &self.inner.base
    }

    pub(super) fn limits(&self) -> Limits {
        self.inner.limits
    }
}

impl AsRef<HttpTransport> for HttpTransport {
    fn as_ref(&self) -> &HttpTransport {
        self
    }
}

impl fmt::Debug for HttpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpTransport")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn permanent_text(config: &HttpConfig) -> String {
        let error = config.validate().expect_err("rejected");
        assert!(
            matches!(error.kind(), nebula_resource::ErrorKind::Permanent),
            "{error:?}"
        );
        error.to_string()
    }

    #[rstest]
    #[case::not_a_url("not a url")]
    #[case::http_remote("http://api.example.com/v1")]
    #[case::userinfo("https://user:hunter2@api.example.com")]
    #[case::query("https://api.example.com/v1?key=hunter2")]
    #[case::fragment("https://api.example.com/v1#hunter2")]
    #[case::other_scheme("ftp://api.example.com")]
    fn a_bad_base_url_is_permanent_and_never_echoed(#[case] base: &str) {
        let text = permanent_text(&HttpConfig::new(base));
        assert!(!text.contains("example.com"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
    }

    #[rstest]
    #[case::localhost("http://localhost:8080")]
    #[case::ipv4("http://127.0.0.1:8080/mount")]
    #[case::ipv6("http://[::1]:8080")]
    #[case::https("https://api.example.com/v3")]
    fn https_or_loopback_http_validates(#[case] base: &str) {
        HttpConfig::new(base).validate().expect("valid");
        HttpTransport::new(&HttpConfig::new(base)).expect("transport builds");
    }

    #[test]
    fn zero_bounds_are_permanent() {
        let base = || HttpConfig::new("https://api.example.com");
        for config in [
            base().with_connect_timeout(Duration::ZERO),
            base().with_request_timeout(Duration::ZERO),
            base().with_read_idle_timeout(Duration::ZERO),
            base().with_max_response_bytes(0),
            base().with_max_stream_bytes(0),
        ] {
            permanent_text(&config);
        }
    }

    #[test]
    fn bad_pem_and_user_agent_are_permanent() {
        let base = || HttpConfig::new("https://api.example.com");
        permanent_text(&base().with_extra_root_certificate_pem("not a certificate"));
        permanent_text(&base().with_user_agent("line\nbreak"));
    }

    #[test]
    fn defaults_fill_absent_fields_and_the_fingerprint_tracks_fields() {
        let parsed: HttpConfig =
            serde_json::from_value(serde_json::json!({ "base_url": "https://api.example.com" }))
                .expect("defaults");
        assert_eq!(parsed, HttpConfig::new("https://api.example.com"));
        assert_eq!(
            parsed.fingerprint(),
            HttpConfig::new("https://api.example.com").fingerprint()
        );
        let before = parsed.fingerprint();
        assert_ne!(
            before,
            parsed
                .with_request_timeout(Duration::from_secs(1))
                .fingerprint()
        );
        assert!(
            serde_json::from_value::<HttpConfig>(serde_json::json!({
                "base_url": "https://api.example.com",
                "proxy": "http://proxy"
            }))
            .is_err(),
            "unknown fields are rejected"
        );
    }

    #[test]
    fn debug_shows_no_url() {
        let config = HttpConfig::new("https://api.example.com/secret-mount");
        let transport = HttpTransport::new(&config).expect("transport");
        for text in [format!("{config:?}"), format!("{transport:?}")] {
            assert!(!text.contains("example.com"), "{text}");
            assert!(!text.contains("secret-mount"), "{text}");
        }
    }
}
