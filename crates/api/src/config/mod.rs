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

mod env;
mod errors;
mod jwt;
pub mod oauth;
mod sub;

pub use errors::ApiConfigError;
pub use jwt::JwtSecret;
pub use oauth::{OAuthEndpoints, OAuthProviderConfig, OAuthProvidersConfig, OIDC_HARDCODED_SCOPES};
pub use sub::{
    AuthApiConfig, AuthBackendKind, CookieConfig, CorsConfig, ExecutionBackendKind,
    ExecutionStoreConfig, IdempotencyApiConfig, IdempotencyBackend, PaginationConfig,
    SmtpEmailConfig, SmtpTlsMode, TlsConfig, VersioningConfig, WebhookApiConfig,
};

use std::{net::SocketAddr, sync::OnceLock, time::Duration};

use serde::{Deserialize, Serialize};

use secrecy::SecretString;

use self::env::{parse_bool_env, parse_positive_u64_env, parse_u64_env, parse_usize_env};
use crate::middleware::idempotency::{
    DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_ENTRIES, DEFAULT_TTL_SECS,
};

/// Default maximum accepted request-body size for REST handlers
/// (1 MiB). Used as the startup default for
/// [`ApiConfig::max_body_size`], which operators can override via
/// the `API_MAX_BODY_SIZE` env var.
///
/// The 1 MiB figure is a guard rail from the 2026-04-19 codebase-quality
/// audit (§"Guard rails" #2) and a pre-condition of REST limits
/// ( §3 #3) for any
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

    /// Idempotency-Key middleware configuration (see idempotency backend).
    #[serde(default)]
    pub idempotency: IdempotencyApiConfig,

    /// Plane-A authentication subsystem configuration.
    ///
    /// Drives the composition root's selection between the dev-only
    /// in-memory `AuthBackend` and the PG-backed `PgAuthBackend`. The
    /// backend selector is bound to `API_AUTH_BACKEND`
    /// (case-insensitive `memory` / `postgres`).
    #[serde(default)]
    pub auth: AuthApiConfig,

    /// Webhook subsystem configuration (webhook activation).
    #[serde(default)]
    pub webhook: WebhookApiConfig,

    /// Execution-store and control-queue backend configuration.
    ///
    /// Drives the composition root's selection between the dev-only
    /// in-memory adapters (default), file-local SQLite, and shared
    /// PostgreSQL. The backend selector is bound to
    /// `API_EXECUTION_BACKEND` (case-insensitive `memory` / `sqlite` /
    /// `postgres`); the SQLite file path is `API_EXECUTION_DB_PATH`.
    #[serde(default)]
    pub execution: ExecutionStoreConfig,

    /// Production SMTP transport configuration for the `EmailPort`.
    ///
    /// `None` keeps the composition root on the dev `EchoSink` (the
    /// local-first default); `Some` triggers the `SmtpEmailPort`
    /// branch in `apps/server::compose`. The sentinel is the
    /// `API_SMTP_HOST` env var — unset means `None`, present means
    /// the full struct must validate or `from_env` returns an error.
    #[serde(default)]
    pub smtp: Option<SmtpEmailConfig>,
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
            .field("auth", &self.auth)
            .field("execution", &self.execution)
            .field("smtp", &self.smtp)
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
        let auth = Self::auth_from_env()?;
        tracing::info!(backend = ?auth.backend, "auth: config loaded");
        let execution = Self::execution_from_env()?;
        tracing::info!(backend = ?execution.backend, db_path = %execution.db_path, "execution-stores: config loaded");
        let smtp = Self::smtp_from_env()?;
        if let Some(cfg) = smtp.as_ref() {
            tracing::info!(
                host = %cfg.host,
                port = cfg.port,
                tls = ?cfg.tls,
                authenticated = cfg.username.is_some(),
                "smtp: config loaded"
            );
        }

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
            auth,
            webhook: Self::webhook_from_env()?,
            execution,
            smtp,
        })
    }

    fn auth_from_env() -> Result<AuthApiConfig, ApiConfigError> {
        let backend = match std::env::var("API_AUTH_BACKEND") {
            Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "memory" => AuthBackendKind::Memory,
                "postgres" => AuthBackendKind::Postgres,
                _ => {
                    return Err(ApiConfigError::ParseEnum {
                        var: "AUTH_BACKEND",
                        raw,
                    });
                },
            },
            Err(_) => AuthBackendKind::Memory,
        };
        // OAuth providers config: scan env vars per OAuthProvider
        // variant for `API_AUTH_OAUTH_<PROVIDER>_*` (T2.2). Returns
        // empty when no provider is declared, so existing operators
        // who never set the env vars keep the legacy behavior
        // (start_oauth returns ProviderNotConfigured per ADR-0085 D-6).
        let oauth = OAuthProvidersConfig::from_env()?;
        Ok(AuthApiConfig { backend, oauth })
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

    fn execution_from_env() -> Result<ExecutionStoreConfig, ApiConfigError> {
        let backend = match std::env::var("API_EXECUTION_BACKEND") {
            Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "memory" => ExecutionBackendKind::Memory,
                "sqlite" => ExecutionBackendKind::Sqlite,
                "postgres" => ExecutionBackendKind::Postgres,
                _ => {
                    return Err(ApiConfigError::ParseEnum {
                        var: "EXECUTION_BACKEND",
                        raw,
                    });
                },
            },
            Err(_) => ExecutionBackendKind::Memory,
        };
        let db_path = std::env::var("API_EXECUTION_DB_PATH")
            .unwrap_or_else(|_| "nebula-server-execution.db".to_string());
        Ok(ExecutionStoreConfig { backend, db_path })
    }

    /// Load the optional SMTP transport config.
    ///
    /// `API_SMTP_HOST` is the sentinel: unset means `None`, present
    /// means every other knob must validate. The validation policy is
    /// fail-CLOSED — silently falling back to `EchoSink` when an
    /// operator who set `API_SMTP_HOST` got the password wrong would
    /// swallow verification mails in production with no diagnostic.
    ///
    /// Validation:
    /// - `API_SMTP_PORT` defaults to `587` (the submission port).
    /// - `API_SMTP_USERNAME` is optional; **but** if it is set,
    ///   `API_SMTP_PASSWORD` MUST also be set (a username without a
    ///   password is never what an operator means, and the SMTP
    ///   handshake will silently fail at first send otherwise).
    /// - `API_SMTP_FROM` is required and must contain `@`. (We do not
    ///   parse the full RFC 5321 mailbox here; the SMTP transport
    ///   rejects invalid mailboxes at first send.)
    /// - `API_SMTP_TLS_MODE` defaults from the port per
    ///   [`SmtpTlsMode::default_for_port`]; `none` is accepted but
    ///   the composition root warns at startup.
    fn smtp_from_env() -> Result<Option<SmtpEmailConfig>, ApiConfigError> {
        // Fail-closed boundary (PR #754 CodeRabbit review): only `Err` —
        // i.e. genuinely unset — falls through to `EchoSink`. `Ok(raw)`
        // where `raw` is empty/whitespace-only is a deliberate (broken)
        // operator value and must surface as a typed startup error
        // instead of a silent dev-mode fallback.
        let host = match std::env::var("API_SMTP_HOST") {
            Ok(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    return Err(ApiConfigError::SmtpHostEmpty);
                }
                trimmed.to_string()
            },
            Err(_) => return Ok(None),
        };

        let port = match std::env::var("API_SMTP_PORT") {
            Ok(raw) => raw
                .parse::<u16>()
                .map_err(|source| ApiConfigError::ParseInt {
                    var: "SMTP_PORT",
                    source,
                })?,
            Err(_) => 587,
        };

        let username = std::env::var("API_SMTP_USERNAME").ok().and_then(|raw| {
            let trimmed = raw.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

        let password = match std::env::var("API_SMTP_PASSWORD") {
            Ok(raw) if !raw.is_empty() => Some(SecretString::from(raw)),
            _ => None,
        };

        // Fail closed: USERNAME without PASSWORD is misconfiguration
        // a silent `None` cannot rescue. Inverse (PASSWORD without
        // USERNAME) is also rejected — unauthenticated relays never
        // need a password and accepting one would mask a typo.
        if username.is_some() != password.is_some() {
            return Err(ApiConfigError::SmtpAuthIncomplete);
        }

        let from_address = std::env::var("API_SMTP_FROM")
            .map_err(|_| ApiConfigError::SmtpFromMissing)
            .and_then(|raw| {
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    Err(ApiConfigError::SmtpFromMissing)
                } else if !trimmed.contains('@') {
                    Err(ApiConfigError::SmtpFromInvalid)
                } else {
                    Ok(trimmed)
                }
            })?;

        let tls = match std::env::var("API_SMTP_TLS_MODE") {
            Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "none" => SmtpTlsMode::None,
                "starttls" | "start_tls" | "start-tls" => SmtpTlsMode::StartTls,
                "implicit" | "smtps" => SmtpTlsMode::Implicit,
                _ => {
                    return Err(ApiConfigError::ParseEnum {
                        var: "SMTP_TLS_MODE",
                        raw,
                    });
                },
            },
            Err(_) => SmtpTlsMode::default_for_port(port),
        };

        Ok(Some(SmtpEmailConfig {
            host,
            port,
            username,
            password,
            from_address,
            tls,
        }))
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
            auth: AuthApiConfig::default(),
            webhook: WebhookApiConfig::default(),
            execution: ExecutionStoreConfig::default(),
            smtp: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::env::tests::env_guard;

    #[test]
    fn from_env_rejects_missing_secret_in_production() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");

        let err = ApiConfig::from_env().expect_err("production + missing must error");
        match err {
            ApiConfigError::MissingJwtSecret(mode) => assert_eq!(mode, "production"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn from_env_rejects_short_secret() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "short");

        let err = ApiConfig::from_env().expect_err("short secret must error");
        match err {
            ApiConfigError::JwtSecretTooShort { got, min } => {
                assert_eq!(got, 5);
                assert_eq!(min, JwtSecret::MIN_BYTES);
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn from_env_rejects_dev_placeholder() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", JwtSecret::DEV_PLACEHOLDER);

        let err = ApiConfig::from_env().expect_err("dev placeholder must error");
        assert!(matches!(err, ApiConfigError::JwtSecretIsDevPlaceholder));
    }

    #[test]
    fn from_env_generates_ephemeral_in_dev() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "development");

        let cfg1 = ApiConfig::from_env().expect("dev mode must succeed");
        let cfg2 = ApiConfig::from_env().expect("dev mode must succeed");

        // Successive loads in one process must reuse the same ephemeral
        // secret so auth state remains stable until restart.
        assert_eq!(cfg1.jwt_secret.as_bytes(), cfg2.jwt_secret.as_bytes());
        assert!(cfg1.jwt_secret.as_bytes().len() >= JwtSecret::MIN_BYTES);
    }

    #[test]
    fn from_env_missing_secret_without_env_fails_closed() {
        let _env = env_guard();

        let err = ApiConfig::from_env().expect_err("unset NEBULA_ENV must fail closed");
        match err {
            ApiConfigError::MissingJwtSecret(mode) => assert_eq!(mode, "production"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn from_env_typo_in_env_mode_does_not_fall_through() {
        // "developmnt" (typo) must NOT be treated as dev. This is the
        // security-lead's explicit ask: unknown env modes fail closed.
        let mut env = env_guard();
        env.set("NEBULA_ENV", "developmnt");

        let err = ApiConfig::from_env().expect_err("typo must not fall through to dev");
        assert!(matches!(err, ApiConfigError::MissingJwtSecret(_)));
    }
}
