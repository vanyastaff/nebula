//! API Configuration
//!
//! Централизованная конфигурация для Nebula API server.
//!
//! ## Security
//!
//! [`ApiConfig`] refuses to produce a runtime config with a missing,
//! short, or well-known placeholder JWT secret. The illegal state
//! (empty / short / known-dev secret in production mode) is
//! unrepresentable at the type level: [`JwtSecret::new`] is the
//! single validation gate, and any `JwtSecret` in hand is guaranteed
//! to be at least [`JwtSecret::MIN_BYTES`] bytes long and not the
//! placeholder literal.
//!
//! There is **no** `impl Default for ApiConfig` — a missing
//! `API_JWT_SECRET` in production mode is a hard startup error, not a
//! soft fallback. Tests must use `ApiConfig::for_test` (gated
//! behind the `test-util` feature.

use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::middleware::idempotency::{
    DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_ENTRIES, DEFAULT_TTL_SECS,
};

/// Validated HS256 signing key.
///
/// Construction via [`JwtSecret::new`] is the ONLY place length and
/// known-bad-value checks live. Any `JwtSecret` in hand is valid.
///
/// `Debug` redacts the secret contents so accidental `{:?}` prints
/// never leak key material into logs.
#[derive(Clone)]
pub struct JwtSecret(Arc<str>);

impl JwtSecret {
    /// Minimum length for HS256. RFC 7518 §3.2 requires "a key of
    /// the same size as the hash output"; for HS256 that is 32 bytes.
    pub const MIN_BYTES: usize = 32;

    /// The well-known development placeholder. Explicitly rejected
    /// even if someone leaks it back in via an env var.
    pub const DEV_PLACEHOLDER: &'static str = "dev-secret-change-in-production";

    /// Validate and wrap a raw secret string.
    ///
    /// # Errors
    ///
    /// - [`ApiConfigError::JwtSecretTooShort`] if the input is shorter than [`Self::MIN_BYTES`]
    ///   bytes.
    /// - [`ApiConfigError::JwtSecretIsDevPlaceholder`] if the input matches the well-known
    ///   development placeholder.
    pub fn new(raw: impl Into<Arc<str>>) -> Result<Self, ApiConfigError> {
        let raw = raw.into();
        if raw.as_ref() == Self::DEV_PLACEHOLDER {
            return Err(ApiConfigError::JwtSecretIsDevPlaceholder);
        }
        if raw.len() < Self::MIN_BYTES {
            return Err(ApiConfigError::JwtSecretTooShort {
                got: raw.len(),
                min: Self::MIN_BYTES,
            });
        }
        Ok(Self(raw))
    }

    /// Return the raw secret bytes for signature verification.
    #[inline]
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Generate a random 32-byte secret, hex-encoded (64 chars).
    ///
    /// Intended for dev-mode ephemeral startup. **Never** call this
    /// in production code paths — it bypasses the guarantee that the
    /// operator has explicitly configured an auth key.
    fn generate_ephemeral() -> Self {
        use rand::RngExt;

        let mut rng = rand::rng();
        let bytes: [u8; 32] = rng.random();
        let mut hex = String::with_capacity(64);
        for b in bytes {
            // Two hex chars per byte — never fails.
            hex.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
            hex.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap_or('0'));
        }
        Self(Arc::from(hex))
    }

    /// Unchecked constructor for the `test-util` feature.
    ///
    /// Only reachable behind `#[cfg(any(test, feature = "test-util"))]`,
    /// so production builds cannot accidentally bypass validation.
    #[cfg(any(test, feature = "test-util"))]
    fn for_test_unchecked(raw: &'static str) -> Self {
        Self(Arc::from(raw))
    }
}

impl std::fmt::Debug for JwtSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JwtSecret([REDACTED])")
    }
}

// Serde: serialize as redacted so accidental config dumps (e.g. via
// `serde_json::to_string(&config)`) never leak the secret. Deserialize
// goes through the validating `new` constructor.
impl Serialize for JwtSecret {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str("[REDACTED]")
    }
}

impl<'de> Deserialize<'de> for JwtSecret {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        JwtSecret::new(raw).map_err(serde::de::Error::custom)
    }
}

/// Typed errors returned by [`ApiConfig::from_env`] and friends.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiConfigError {
    /// `API_JWT_SECRET` unset in a non-dev `NEBULA_ENV`.
    #[error("API_JWT_SECRET is required in non-dev mode (NEBULA_ENV={0})")]
    MissingJwtSecret(String),

    /// `API_JWT_SECRET` is shorter than [`JwtSecret::MIN_BYTES`].
    #[error("API_JWT_SECRET is too short ({got} bytes, minimum {min})")]
    JwtSecretTooShort {
        /// Length of the provided secret in bytes.
        got: usize,
        /// Required minimum in bytes.
        min: usize,
    },

    /// `API_JWT_SECRET` is literally the well-known dev placeholder.
    #[error("API_JWT_SECRET matches the well-known development placeholder — refusing to start")]
    JwtSecretIsDevPlaceholder,

    /// `API_BIND_ADDRESS` failed to parse.
    #[error("API_BIND_ADDRESS invalid")]
    BindAddress(#[source] std::net::AddrParseError),

    /// An `API_*` integer env var failed to parse.
    #[error("API_{var} invalid")]
    ParseInt {
        /// Name of the env var suffix after `API_`.
        var: &'static str,
        #[source]
        /// Underlying parse error.
        source: std::num::ParseIntError,
    },

    /// An `API_*` boolean env var failed to parse.
    #[error("API_{var} invalid")]
    ParseBool {
        /// Name of the env var suffix after `API_`.
        var: &'static str,
        #[source]
        /// Underlying parse error.
        source: std::str::ParseBoolError,
    },

    /// An `API_*` enum-typed env var failed to match a known variant.
    ///
    /// Used by [`ApiConfig::from_env`] for `API_IDEMPOTENCY_BACKEND` and
    /// other enum-shaped knobs added in future revisions. Carries the raw
    /// value so operator-facing logs can show the typo without re-reading
    /// the env.
    #[error("API_{var} invalid: {raw:?}")]
    ParseEnum {
        /// Name of the env var suffix after `API_`.
        var: &'static str,
        /// The raw value the operator supplied (already-failed parse).
        raw: String,
    },

    /// An `API_*` numeric env var must be strictly positive but the
    /// operator supplied `0`.
    ///
    /// Used for knobs whose zero value would silently disable a hard
    /// invariant — for example `API_IDEMPOTENCY_TTL_SECS=0` would build
    /// a cache where every entry expires immediately, silently turning
    /// off replay protection. Surface the misconfiguration at startup
    /// instead of letting the runtime degrade.
    #[error("API_{var} must be > 0")]
    ZeroValue {
        /// Name of the env var suffix after `API_`.
        var: &'static str,
    },
}

// ── Config sub-structs ─────────────────────────────────────────────────────

/// TLS configuration for the API server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Whether TLS termination is enabled.
    pub enabled: bool,
    /// Path to the TLS certificate file.
    pub cert_path: Option<String>,
    /// Path to the TLS private key file.
    pub key_path: Option<String>,
    /// Whether ACME (Let's Encrypt) certificate provisioning is enabled.
    pub acme_enabled: bool,
    /// ACME directory URL (e.g. Let's Encrypt production/staging).
    pub acme_directory: Option<String>,
    /// Contact email for ACME certificate notifications.
    pub acme_email: Option<String>,
}

/// Cookie configuration for session handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieConfig {
    /// Domain scope for session cookies.
    pub domain: String,
    /// Whether to set the `Secure` flag (HTTPS-only).
    pub secure: bool,
    /// `SameSite` attribute: `"lax"`, `"strict"`, or `"none"`.
    pub same_site: String,
    /// Maximum session age in seconds (default: 604 800 = 7 days).
    pub session_max_age_secs: u64,
}

impl Default for CookieConfig {
    fn default() -> Self {
        Self {
            domain: ".localhost".to_string(),
            secure: false,
            same_site: "lax".to_string(),
            session_max_age_secs: 604_800,
        }
    }
}

/// CORS configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    /// Allowed origins (e.g. `["https://app.nebula.dev"]`; `["*"]` for dev).
    pub allowed_origins: Vec<String>,
    /// Whether to allow credentials (cookies, auth headers).
    pub allow_credentials: bool,
    /// `Access-Control-Max-Age` in seconds.
    pub max_age_secs: u64,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: vec!["*".to_string()],
            allow_credentials: false,
            max_age_secs: 3600,
        }
    }
}

/// API versioning configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersioningConfig {
    /// Currently supported API versions (e.g. `["v1"]`).
    pub supported_versions: Vec<String>,
    /// Deprecated but still served versions.
    pub deprecated_versions: Vec<String>,
}

impl Default for VersioningConfig {
    fn default() -> Self {
        Self {
            supported_versions: vec!["v1".to_string()],
            deprecated_versions: Vec::new(),
        }
    }
}

/// Idempotency-Key middleware configuration.
///
/// See ADR-0048 for the backend selection contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyApiConfig {
    /// Storage backend for cached idempotent responses.
    ///
    /// `Memory` is correct for dev / single-process tests but loses dedup
    /// state across restart and across runners — operators must select
    /// `Postgres` for any deployment that runs more than one API replica
    /// or expects stable behaviour across restarts.
    pub backend: IdempotencyBackend,

    /// Cached-entry lifetime in seconds. Defaults to
    /// [`DEFAULT_TTL_SECS`] (24 h, the IETF draft recommendation).
    pub ttl_secs: u64,

    /// Maximum number of cached entries (in-memory backend only —
    /// `moka::Cache` honours this as a hard cap; the PG backend treats
    /// `expires_at` as the eviction signal).
    pub max_entries: u64,

    /// Maximum buffered request body size (bytes) eligible for caching.
    /// Requests larger than this pass through without idempotency tracking.
    pub max_request_body_bytes: usize,

    /// Maximum buffered response body size (bytes) eligible for caching.
    /// Larger responses are returned to the caller but never cached.
    pub max_response_body_bytes: usize,

    /// PG-only: cadence for the background expired-row sweep.
    ///
    /// `0` disables the sweep (dev / single-process runs); the memory
    /// backend ignores this field because `moka` evicts on TTL. A value
    /// `< 60` triggers a startup `tracing::warn!` (see ADR-0048
    /// "sweep cadence sanity floor") but is not rejected.
    pub sweep_interval_secs: u64,
}

/// Backend selection for the idempotency store.
///
/// See ADR-0048 for the decision rationale and the fail-closed contract
/// in the composition root (selecting `Postgres` without a configured
/// `DATABASE_URL` is a hard startup error).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IdempotencyBackend {
    /// Process-local cache (`moka::future::Cache`). Correct for dev and
    /// for single-process tests; loses state on restart and cannot be
    /// shared across runners.
    Memory,
    /// PostgreSQL-backed durable store (see ADR-0048). Survives restart
    /// and is shared across runners that point at the same database.
    Postgres,
}

impl IdempotencyApiConfig {
    /// Default TTL applied when [`from_env`](ApiConfig::from_env) does not
    /// see `API_IDEMPOTENCY_TTL_SECS`. Defined here (rather than reusing
    /// the middleware constant directly) so future tuning can diverge
    /// without churning every test.
    pub const DEFAULT_SWEEP_INTERVAL_SECS: u64 = 300;
}

impl Default for IdempotencyApiConfig {
    fn default() -> Self {
        Self {
            backend: IdempotencyBackend::Memory,
            ttl_secs: DEFAULT_TTL_SECS,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_request_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_response_body_bytes: DEFAULT_MAX_BODY_BYTES,
            sweep_interval_secs: Self::DEFAULT_SWEEP_INTERVAL_SECS,
        }
    }
}

/// Webhook subsystem configuration (M3.3 / ADR-0049).
///
/// Controls how the slug-routed webhook surface boots. Default is
/// `bootstrap_from_storage = true` so production deployments wire
/// activation rows on startup; tests opt out by setting the field to
/// `false` and seeding the transport directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookApiConfig {
    /// When `true`, the composition root invokes
    /// [`crate::services::webhook::bootstrap_webhook_activations`]
    /// before `build_app` to populate the transport's slug map from
    /// `WebhookActivationRepo`. When `false`, the slug map starts
    /// empty and only programmatic activations are dispatched.
    ///
    /// Env var: `API_WEBHOOK_BOOTSTRAP_FROM_STORAGE`
    /// (`true` / `false` / `1` / `0`; default `true`).
    pub bootstrap_from_storage: bool,
}

impl Default for WebhookApiConfig {
    fn default() -> Self {
        Self {
            bootstrap_from_storage: true,
        }
    }
}

/// Pagination configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationConfig {
    /// Default page size when the caller omits `limit`.
    pub default_limit: u32,
    /// Hard upper bound on `limit`.
    pub max_limit: u32,
}

impl Default for PaginationConfig {
    fn default() -> Self {
        Self {
            default_limit: 50,
            max_limit: 500,
        }
    }
}

/// Default maximum accepted request-body size for REST handlers
/// (1 MiB). Used as the startup default for
/// [`ApiConfig::max_body_size`], which operators can override via
/// the `API_MAX_BODY_SIZE` env var.
///
/// The 1 MiB figure is a guard rail from the 2026-04-19 audit
/// (`docs/audit/2026-04-19-codebase-quality-audit.md` §"Guard rails"
/// #2) and a pre-condition of ADR-0020
/// (`docs/adr/0020-library-first-gtm.md` §3 #3) for any
/// composition-root binary.
///
/// The webhook transport applies its own cap on its sub-router
/// (`crates/api/src/webhook/transport.rs`); this constant covers only
/// the REST surface (`/workflows`, `/credentials`, …). Operators can
/// grep this symbol to find the default and raise it per deployment
/// via the env var if a specific workload genuinely needs larger
/// payloads.
pub const REST_BODY_LIMIT_BYTES: usize = 1024 * 1024;

/// API Server Configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Host and port to bind (e.g. "0.0.0.0:8080")
    pub bind_address: SocketAddr,

    /// Request timeout
    pub request_timeout: Duration,

    /// Maximum request body size (bytes) for REST endpoints.
    ///
    /// Wired into the REST router as `axum::extract::DefaultBodyLimit`
    /// by [`crate::app::build_app`]; does **not** apply to webhook
    /// ingress, which has its own cap. Defaults to
    /// [`REST_BODY_LIMIT_BYTES`] (1 MiB) and is overridable via
    /// `API_MAX_BODY_SIZE`.
    pub max_body_size: usize,

    /// CORS allowed origins
    pub cors_allowed_origins: Vec<String>,

    /// Enable compression (gzip, brotli, zstd)
    pub enable_compression: bool,

    /// Enable request tracing
    pub enable_tracing: bool,

    /// JWT secret for authentication.
    ///
    /// Wrapped in [`JwtSecret`] so the illegal state (empty / short /
    /// known-dev placeholder) is unrepresentable.
    pub jwt_secret: JwtSecret,

    /// Rate limiting: requests per second per IP
    pub rate_limit_per_second: u32,

    /// Static API keys accepted via `X-API-Key` header.
    ///
    /// Each key must have the `nbl_sk_` prefix. Keys are compared in constant
    /// time to prevent timing attacks. An empty list disables API key auth.
    #[serde(default)]
    pub api_keys: Vec<String>,

    /// Externally-reachable base URL of this API server.
    pub public_url: String,

    /// Per-request timeout in seconds (used by middleware layers).
    pub request_timeout_secs: u64,

    /// Header name used for request-id propagation.
    pub request_id_header: String,

    /// TLS termination settings.
    pub tls: TlsConfig,

    /// Session-cookie settings.
    pub cookies: CookieConfig,

    /// Structured CORS configuration.
    ///
    /// Supersedes the flat `cors_allowed_origins` list; both remain so
    /// existing `from_env` callers keep working.
    pub cors_config: CorsConfig,

    /// API versioning metadata.
    pub versioning: VersioningConfig,

    /// Pagination defaults and caps.
    pub pagination: PaginationConfig,

    /// Idempotency-Key middleware configuration (see ADR-0048).
    #[serde(default)]
    pub idempotency: IdempotencyApiConfig,

    /// Webhook subsystem configuration (M3.3 / ADR-0049).
    #[serde(default)]
    pub webhook: WebhookApiConfig,
}

impl std::fmt::Debug for ApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiConfig")
            .field("bind_address", &self.bind_address)
            .field("request_timeout", &self.request_timeout)
            .field("max_body_size", &self.max_body_size)
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("enable_compression", &self.enable_compression)
            .field("enable_tracing", &self.enable_tracing)
            .field("jwt_secret", &self.jwt_secret)
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("api_keys", &"[REDACTED]")
            .field("public_url", &self.public_url)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("request_id_header", &self.request_id_header)
            .field("tls", &self.tls)
            .field("cookies", &self.cookies)
            .field("cors_config", &self.cors_config)
            .field("versioning", &self.versioning)
            .field("pagination", &self.pagination)
            .field("idempotency", &self.idempotency)
            .finish()
    }
}

impl ApiConfig {
    fn dev_ephemeral_secret(env_mode: &str) -> JwtSecret {
        static DEV_SECRET: OnceLock<JwtSecret> = OnceLock::new();
        DEV_SECRET
            .get_or_init(|| {
                tracing::warn!(
                    nebula_env = %env_mode,
                    "API_JWT_SECRET unset; generated process-scoped ephemeral secret for dev mode. \
                     Tokens will be invalidated on restart."
                );
                JwtSecret::generate_ephemeral()
            })
            .clone()
    }

    /// Load configuration from environment variables.
    ///
    /// Honours `NEBULA_ENV` to decide whether a missing
    /// `API_JWT_SECRET` is tolerable. In `development` / `dev` /
    /// `local` the loader generates a random per-process ephemeral
    /// secret and logs a single warning. In any other mode, a missing
    /// `API_JWT_SECRET` is a hard error — the server refuses to
    /// start. This is intentional: a publicly-known HS256 key is a
    /// full auth bypass.
    ///
    /// # Errors
    ///
    /// Returns [`ApiConfigError`] on:
    ///
    /// - missing `API_JWT_SECRET` outside dev mode
    /// - `API_JWT_SECRET` shorter than [`JwtSecret::MIN_BYTES`]
    /// - `API_JWT_SECRET` matching the well-known dev placeholder
    /// - malformed `API_BIND_ADDRESS`, `API_REQUEST_TIMEOUT`, `API_MAX_BODY_SIZE`,
    ///   `API_ENABLE_COMPRESSION`, `API_ENABLE_TRACING`, or `API_RATE_LIMIT`
    pub fn from_env() -> Result<Self, ApiConfigError> {
        let env_mode = std::env::var("NEBULA_ENV").unwrap_or_else(|_| "production".to_string());
        let is_dev = matches!(env_mode.as_str(), "development" | "dev" | "local");

        let jwt_secret = match std::env::var("API_JWT_SECRET") {
            Ok(raw) => JwtSecret::new(raw)?,
            Err(_) if is_dev => Self::dev_ephemeral_secret(&env_mode),
            Err(_) => return Err(ApiConfigError::MissingJwtSecret(env_mode)),
        };

        let bind_address = std::env::var("API_BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()
            .map_err(ApiConfigError::BindAddress)?;

        let request_timeout = std::env::var("API_REQUEST_TIMEOUT")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|source| ApiConfigError::ParseInt {
                var: "REQUEST_TIMEOUT",
                source,
            })?;

        let max_body_size = std::env::var("API_MAX_BODY_SIZE")
            .ok()
            .map(|raw| {
                raw.parse().map_err(|source| ApiConfigError::ParseInt {
                    var: "MAX_BODY_SIZE",
                    source,
                })
            })
            .transpose()?
            .unwrap_or(REST_BODY_LIMIT_BYTES);

        let cors_allowed_origins: Vec<String> = std::env::var("API_CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let enable_compression = std::env::var("API_ENABLE_COMPRESSION")
            .unwrap_or_else(|_| "true".to_string())
            .parse()
            .map_err(|source| ApiConfigError::ParseBool {
                var: "ENABLE_COMPRESSION",
                source,
            })?;

        let enable_tracing = std::env::var("API_ENABLE_TRACING")
            .unwrap_or_else(|_| "true".to_string())
            .parse()
            .map_err(|source| ApiConfigError::ParseBool {
                var: "ENABLE_TRACING",
                source,
            })?;

        let rate_limit_per_second = std::env::var("API_RATE_LIMIT")
            .unwrap_or_else(|_| "100".to_string())
            .parse()
            .map_err(|source| ApiConfigError::ParseInt {
                var: "RATE_LIMIT",
                source,
            })?;

        // API keys: comma-separated list in `API_KEYS` env var.
        let api_keys = std::env::var("API_KEYS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        let public_url =
            std::env::var("API_PUBLIC_URL").unwrap_or_else(|_| format!("http://{bind_address}"));

        let request_timeout_secs = request_timeout.as_secs();

        let request_id_header =
            std::env::var("API_REQUEST_ID_HEADER").unwrap_or_else(|_| "x-request-id".to_string());

        let idempotency = Self::idempotency_from_env()?;
        tracing::info!(
            backend = ?idempotency.backend,
            ttl_secs = idempotency.ttl_secs,
            max_entries = idempotency.max_entries,
            sweep_interval_secs = idempotency.sweep_interval_secs,
            "idempotency: config loaded"
        );

        Ok(Self {
            bind_address,
            request_timeout,
            max_body_size,
            cors_allowed_origins: cors_allowed_origins.clone(),
            enable_compression,
            enable_tracing,
            jwt_secret,
            rate_limit_per_second,
            api_keys,
            public_url,
            request_timeout_secs,
            request_id_header,
            tls: TlsConfig::default(),
            cookies: CookieConfig::default(),
            cors_config: CorsConfig {
                allowed_origins: cors_allowed_origins,
                ..CorsConfig::default()
            },
            versioning: VersioningConfig::default(),
            pagination: PaginationConfig::default(),
            idempotency,
            webhook: Self::webhook_from_env()?,
        })
    }

    fn idempotency_from_env() -> Result<IdempotencyApiConfig, ApiConfigError> {
        let backend = match std::env::var("API_IDEMPOTENCY_BACKEND") {
            Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "memory" => IdempotencyBackend::Memory,
                "postgres" => IdempotencyBackend::Postgres,
                _ => {
                    return Err(ApiConfigError::ParseEnum {
                        var: "IDEMPOTENCY_BACKEND",
                        raw,
                    });
                },
            },
            Err(_) => IdempotencyBackend::Memory,
        };

        let ttl_secs = parse_positive_u64_env("IDEMPOTENCY_TTL_SECS", DEFAULT_TTL_SECS)?;
        let max_entries = parse_positive_u64_env("IDEMPOTENCY_MAX_ENTRIES", DEFAULT_MAX_ENTRIES)?;
        let max_request_body_bytes =
            parse_usize_env("IDEMPOTENCY_MAX_REQUEST_BODY_BYTES", DEFAULT_MAX_BODY_BYTES)?;
        let max_response_body_bytes = parse_usize_env(
            "IDEMPOTENCY_MAX_RESPONSE_BODY_BYTES",
            DEFAULT_MAX_BODY_BYTES,
        )?;
        // sweep_interval_secs uses the non-positive variant — `0`
        // disables the sweep (dev / single-process runs).
        let sweep_interval_secs = parse_u64_env(
            "IDEMPOTENCY_SWEEP_INTERVAL_SECS",
            IdempotencyApiConfig::DEFAULT_SWEEP_INTERVAL_SECS,
        )?;

        Ok(IdempotencyApiConfig {
            backend,
            ttl_secs,
            max_entries,
            max_request_body_bytes,
            max_response_body_bytes,
            sweep_interval_secs,
        })
    }

    fn webhook_from_env() -> Result<WebhookApiConfig, ApiConfigError> {
        let bootstrap_from_storage = parse_bool_env("WEBHOOK_BOOTSTRAP_FROM_STORAGE", true)?;
        Ok(WebhookApiConfig {
            bootstrap_from_storage,
        })
    }

    /// Build a config suitable for integration tests.
    ///
    /// Uses a fixed, obviously-test-only secret that bypasses the
    /// validation gate. Only reachable when the `test-util` feature
    /// is enabled (or under `#[cfg(test)]` of this crate itself).
    /// Production builds never see this path.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn for_test() -> Self {
        Self {
            bind_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            request_timeout: Duration::from_secs(30),
            max_body_size: REST_BODY_LIMIT_BYTES,
            cors_allowed_origins: vec!["*".to_string()],
            enable_compression: false,
            enable_tracing: false,
            jwt_secret: JwtSecret::for_test_unchecked(
                "test-secret-for-integration-tests-0123456789",
            ),
            rate_limit_per_second: 100,
            api_keys: Vec::new(),
            public_url: "http://127.0.0.1:0".to_string(),
            request_timeout_secs: 30,
            request_id_header: "x-request-id".to_string(),
            tls: TlsConfig::default(),
            cookies: CookieConfig::default(),
            cors_config: CorsConfig::default(),
            versioning: VersioningConfig::default(),
            pagination: PaginationConfig::default(),
            idempotency: IdempotencyApiConfig::default(),
            webhook: WebhookApiConfig::default(),
        }
    }
}

fn parse_u64_env(suffix: &'static str, default: u64) -> Result<u64, ApiConfigError> {
    match std::env::var(format!("API_{suffix}")) {
        Ok(raw) => raw.parse().map_err(|source| ApiConfigError::ParseInt {
            var: suffix,
            source,
        }),
        Err(_) => Ok(default),
    }
}

/// Parse a strictly-positive `u64` from `API_<suffix>`. Rejects `0`
/// with [`ApiConfigError::ZeroValue`] — caller specifies a knob whose
/// zero value would silently disable a hard invariant.
fn parse_positive_u64_env(suffix: &'static str, default: u64) -> Result<u64, ApiConfigError> {
    let value = parse_u64_env(suffix, default)?;
    if value == 0 {
        return Err(ApiConfigError::ZeroValue { var: suffix });
    }
    Ok(value)
}

fn parse_usize_env(suffix: &'static str, default: usize) -> Result<usize, ApiConfigError> {
    match std::env::var(format!("API_{suffix}")) {
        Ok(raw) => raw.parse().map_err(|source| ApiConfigError::ParseInt {
            var: suffix,
            source,
        }),
        Err(_) => Ok(default),
    }
}

/// Parse a boolean from `API_<suffix>`. Accepts `true` / `false` /
/// `1` / `0` (case-insensitive). Empty or unset → `default`.
fn parse_bool_env(suffix: &'static str, default: bool) -> Result<bool, ApiConfigError> {
    match std::env::var(format!("API_{suffix}")) {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(ApiConfigError::ParseEnum { var: suffix, raw }),
        },
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
#[allow(
    unsafe_code,
    reason = "env::{set_var, remove_var} are unsafe under edition 2024"
)]
mod tests {
    use super::*;

    /// Serializes env-var manipulation across tests in this module so
    /// parallel nextest execution does not clobber shared state. We do
    /// not pull in `temp-env`/`serial_test` just for this.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Clears every env var `from_env` reads. Must be called inside
    /// the lock.
    fn clear_env() {
        // SAFETY: protected by `env_lock()` — caller holds the guard.
        unsafe {
            for key in [
                "NEBULA_ENV",
                "API_JWT_SECRET",
                "API_BIND_ADDRESS",
                "API_REQUEST_TIMEOUT",
                "API_MAX_BODY_SIZE",
                "API_CORS_ORIGINS",
                "API_ENABLE_COMPRESSION",
                "API_ENABLE_TRACING",
                "API_RATE_LIMIT",
                "API_KEYS",
                "API_IDEMPOTENCY_BACKEND",
                "API_IDEMPOTENCY_TTL_SECS",
                "API_IDEMPOTENCY_MAX_ENTRIES",
                "API_IDEMPOTENCY_MAX_REQUEST_BODY_BYTES",
                "API_IDEMPOTENCY_MAX_RESPONSE_BODY_BYTES",
                "API_IDEMPOTENCY_SWEEP_INTERVAL_SECS",
            ] {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    fn from_env_rejects_missing_secret_in_production() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe { std::env::set_var("NEBULA_ENV", "production") };

        let err = ApiConfig::from_env().expect_err("production + missing must error");
        match err {
            ApiConfigError::MissingJwtSecret(mode) => assert_eq!(mode, "production"),
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn from_env_rejects_short_secret() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "short");
        }

        let err = ApiConfig::from_env().expect_err("short secret must error");
        match err {
            ApiConfigError::JwtSecretTooShort { got, min } => {
                assert_eq!(got, 5);
                assert_eq!(min, JwtSecret::MIN_BYTES);
            },
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn from_env_rejects_dev_placeholder() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", JwtSecret::DEV_PLACEHOLDER);
        }

        let err = ApiConfig::from_env().expect_err("dev placeholder must error");
        assert!(matches!(err, ApiConfigError::JwtSecretIsDevPlaceholder));

        clear_env();
    }

    #[test]
    fn from_env_generates_ephemeral_in_dev() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe { std::env::set_var("NEBULA_ENV", "development") };

        let cfg1 = ApiConfig::from_env().expect("dev mode must succeed");
        let cfg2 = ApiConfig::from_env().expect("dev mode must succeed");

        // Successive loads in one process must reuse the same ephemeral
        // secret so auth state remains stable until restart.
        assert_eq!(cfg1.jwt_secret.as_bytes(), cfg2.jwt_secret.as_bytes());
        assert!(cfg1.jwt_secret.as_bytes().len() >= JwtSecret::MIN_BYTES);

        clear_env();
    }

    #[test]
    fn from_env_missing_secret_without_env_fails_closed() {
        let _g = env_lock();
        clear_env();

        let err = ApiConfig::from_env().expect_err("unset NEBULA_ENV must fail closed");
        match err {
            ApiConfigError::MissingJwtSecret(mode) => assert_eq!(mode, "production"),
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn from_env_typo_in_env_mode_does_not_fall_through() {
        // "developmnt" (typo) must NOT be treated as dev. This is the
        // security-lead's explicit ask: unknown env modes fail closed.
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe { std::env::set_var("NEBULA_ENV", "developmnt") };

        let err = ApiConfig::from_env().expect_err("typo must not fall through to dev");
        assert!(matches!(err, ApiConfigError::MissingJwtSecret(_)));

        clear_env();
    }

    #[test]
    fn jwt_secret_debug_is_redacted() {
        let secret = JwtSecret::new("this-is-a-32-byte-minimum-secret!!".to_string()).unwrap();
        let formatted = format!("{secret:?}");
        assert!(formatted.contains("REDACTED"));
        assert!(!formatted.contains("this-is-a-32-byte"));
    }

    #[test]
    fn jwt_secret_new_rejects_placeholder() {
        let err = JwtSecret::new(JwtSecret::DEV_PLACEHOLDER.to_string())
            .expect_err("placeholder must be rejected");
        assert!(matches!(err, ApiConfigError::JwtSecretIsDevPlaceholder));
    }

    #[test]
    fn from_env_idempotency_defaults_to_memory() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        }

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Memory);
        assert_eq!(cfg.idempotency.ttl_secs, DEFAULT_TTL_SECS);
        assert_eq!(cfg.idempotency.max_entries, DEFAULT_MAX_ENTRIES);
        assert_eq!(
            cfg.idempotency.sweep_interval_secs,
            IdempotencyApiConfig::DEFAULT_SWEEP_INTERVAL_SECS
        );

        clear_env();
    }

    #[test]
    fn from_env_idempotency_accepts_postgres_backend() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_IDEMPOTENCY_BACKEND", "postgres");
            std::env::set_var("API_IDEMPOTENCY_TTL_SECS", "3600");
            std::env::set_var("API_IDEMPOTENCY_SWEEP_INTERVAL_SECS", "120");
        }

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Postgres);
        assert_eq!(cfg.idempotency.ttl_secs, 3600);
        assert_eq!(cfg.idempotency.sweep_interval_secs, 120);

        clear_env();
    }

    #[test]
    fn from_env_idempotency_backend_is_case_insensitive() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_IDEMPOTENCY_BACKEND", "POSTGRES");
        }

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Postgres);

        clear_env();
    }

    #[test]
    fn from_env_idempotency_rejects_unknown_backend() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_IDEMPOTENCY_BACKEND", "redis");
        }

        let err = ApiConfig::from_env().expect_err("unknown backend must error");
        match err {
            ApiConfigError::ParseEnum { var, raw } => {
                assert_eq!(var, "IDEMPOTENCY_BACKEND");
                assert_eq!(raw, "redis");
            },
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn from_env_idempotency_rejects_invalid_ttl() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_IDEMPOTENCY_TTL_SECS", "not-a-number");
        }

        let err = ApiConfig::from_env().expect_err("non-numeric TTL must error");
        match err {
            ApiConfigError::ParseInt { var, .. } => {
                assert_eq!(var, "IDEMPOTENCY_TTL_SECS");
            },
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn for_test_idempotency_defaults_to_memory() {
        let cfg = ApiConfig::for_test();
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Memory);
        assert!(cfg.idempotency.ttl_secs > 0);
    }

    #[test]
    fn from_env_idempotency_rejects_zero_ttl_secs() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_IDEMPOTENCY_TTL_SECS", "0");
        }

        let err = ApiConfig::from_env().expect_err("ttl_secs=0 must error");
        match err {
            ApiConfigError::ZeroValue { var } => assert_eq!(var, "IDEMPOTENCY_TTL_SECS"),
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn from_env_idempotency_rejects_zero_max_entries() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_IDEMPOTENCY_MAX_ENTRIES", "0");
        }

        let err = ApiConfig::from_env().expect_err("max_entries=0 must error");
        match err {
            ApiConfigError::ZeroValue { var } => assert_eq!(var, "IDEMPOTENCY_MAX_ENTRIES"),
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }
}
