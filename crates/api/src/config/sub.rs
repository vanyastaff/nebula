use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::middleware::idempotency::{
    DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_ENTRIES, DEFAULT_TTL_SECS,
};

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
/// See idempotency backend for the backend selection contract.
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
    /// `< 60` triggers a startup `tracing::warn!` (see idempotency backend
    /// "sweep cadence sanity floor") but is not rejected.
    pub sweep_interval_secs: u64,
}

/// Backend selection for the idempotency store.
///
/// See idempotency backend for the decision rationale and the fail-closed contract
/// in the composition root (selecting `Postgres` without a configured
/// `DATABASE_URL` is a hard startup error).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IdempotencyBackend {
    /// Process-local cache (`moka::future::Cache`). Correct for dev and
    /// for single-process tests; loses state on restart and cannot be
    /// shared across runners.
    Memory,
    /// PostgreSQL-backed durable store (see idempotency backend). Survives restart
    /// and is shared across runners that point at the same database.
    Postgres,
}

impl IdempotencyApiConfig {
    /// Default TTL applied when [`from_env`](super::ApiConfig::from_env) does not
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

/// Authentication backend selection.
///
/// Drives composition-root selection between the dev-only
/// [`InMemoryAuthBackend`] (the production-quality default with Argon2id
/// passwords, RFC 6238 TOTP, and SHA-256 PAT lookup but per-process
/// `DashMap` state that is lost on restart) and the durable PG-backed
/// `PgAuthBackend`.
///
/// The composition root MUST fail closed when [`AuthBackendKind::Postgres`]
/// is selected without a configured `DATABASE_URL`, mirroring the
/// idempotency backend selector — a publicly-known auth bypass via a
/// missing identity store is exactly what this knob exists to prevent.
///
/// [`InMemoryAuthBackend`]: crate::domain::auth::backend::InMemoryAuthBackend
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AuthBackendKind {
    /// Process-local `DashMap` backend. Correct for dev / tests /
    /// `simple_server`; loses identity state on restart and cannot be
    /// shared across replicas.
    #[default]
    Memory,
    /// Durable PostgreSQL-backed identity backend. Survives restart
    /// and is shared across replicas that point at the same database.
    Postgres,
}

/// Plane-A authentication subsystem configuration.
///
/// Parallel to [`IdempotencyApiConfig`]: just the [`AuthBackendKind`]
/// selector for now. Future PRs (lockout knobs, session TTL overrides,
/// MFA enforcement) extend this struct without changing the env-binding
/// shape (`API_AUTH_*` prefix, matching the existing `API_IDEMPOTENCY_*`
/// convention).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthApiConfig {
    /// Selected identity backend. Defaults to [`AuthBackendKind::Memory`]
    /// so a missing `API_AUTH_BACKEND` keeps current dev behaviour; the
    /// composition root flips this for production deployments via
    /// `API_AUTH_BACKEND=postgres`.
    pub backend: AuthBackendKind,

    /// Operator-supplied OAuth identity-provider configuration (Plane A).
    ///
    /// Empty by default — no OAuth providers declared, the
    /// `/auth/oauth/{provider}/start` endpoints return
    /// `AuthError::ProviderNotConfigured` per ADR-0085 D-6. When
    /// non-empty, every entry is validated synchronously at boot per
    /// REQ-compose-001 (PR-2 T2.8).
    #[serde(default)]
    pub oauth: super::oauth::OAuthProvidersConfig,
}

/// Webhook subsystem configuration (webhook activation).
///
/// Controls how the slug-routed webhook surface boots. Default is
/// `bootstrap_from_storage = true` so production deployments wire
/// activation rows on startup; tests opt out by setting the field to
/// `false` and seeding the transport directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookApiConfig {
    /// When `true`, the composition root invokes
    /// `bootstrap_webhook_activations` before `build_app` to validate
    /// the active rows in the port store (`WebhookActivationStore`) —
    /// confirming each row's factory, secret, and spec can be resolved
    /// so startup logs surface misconfiguration early (ADR-0096).
    /// Dispatch routing onto the in-memory map is deferred to U-D1.4b.
    /// When `false`, the validation step is skipped entirely.
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

/// TLS posture for the SMTP transport.
///
/// Mirrors the SMTP submission-port convention: port 587 negotiates TLS
/// via `STARTTLS` on top of an initially-plaintext connection (the
/// modern submission default), port 465 uses TLS from the first byte
/// (legacy "SMTPS" / implicit TLS), and `None` is a plaintext build
/// reserved for in-cluster dev only — the composition root emits a
/// `tracing::warn!` when this variant is selected so an operator who
/// reaches for `None` in production sees it in the startup log.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmtpTlsMode {
    /// Plaintext SMTP. **Dev only.** Composition root warns on startup.
    None,
    /// Opportunistic upgrade via `STARTTLS` — the standard for the
    /// SMTP submission port (587).
    StartTls,
    /// Implicit TLS from the first byte — the standard for the legacy
    /// SMTPS port (465).
    Implicit,
}

impl SmtpTlsMode {
    /// Pick a sensible default TLS mode for `port`.
    ///
    /// - `465` → [`Self::Implicit`] (SMTPS)
    /// - `587` → [`Self::StartTls`] (submission)
    /// - anything else → [`Self::None`] (plaintext; composition root warns)
    #[must_use]
    pub const fn default_for_port(port: u16) -> Self {
        match port {
            465 => Self::Implicit,
            587 => Self::StartTls,
            _ => Self::None,
        }
    }
}

/// Production SMTP transport configuration for the `EmailPort`.
///
/// Populated only when the operator sets `API_SMTP_HOST`; absence keeps
/// the composition root on the dev-only `EchoSink` so the local-first
/// `simple_server` boot path stays unchanged. When present, the
/// composition root constructs an `SmtpEmailPort` in `apps/server` and
/// fails CLOSED on any malformed value (missing password while
/// `username` is set, missing `from_address`, etc.) — silently falling
/// back to `EchoSink` would silently swallow verification mails in a
/// deployment that explicitly requested SMTP.
///
/// `password` is wrapped in `secrecy::SecretString` so the auto-derived
/// `Debug` redacts the value and zeroizes the buffer on drop; the
/// `Display` impl on `EmailError` already prints `[redacted]` for the
/// rejected-address path, and the SMTP transport mapping mirrors that
/// discipline so no credential ever reaches a `tracing::error!` line or
/// a `problem-details` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpEmailConfig {
    /// SMTP server host (e.g. `"smtp.example.com"`).
    pub host: String,
    /// SMTP server port. Conventionally `587` (submission/STARTTLS),
    /// `465` (SMTPS/implicit TLS), or `25` (plaintext relay).
    pub port: u16,
    /// SASL username. `None` means an unauthenticated relay — useful in
    /// in-cluster dev where the SMTP server trusts the source IP; rare
    /// in production. When `Some`, `password` MUST also be `Some` or
    /// the env-binding parser rejects the config at startup.
    pub username: Option<String>,
    /// SASL password. Wrapped in `SecretString` so `Debug` redacts the
    /// value and zeroizes on drop. Required iff `username` is `Some`.
    ///
    /// `#[serde(skip)]` because the field is only ever populated from
    /// the environment (`API_SMTP_PASSWORD`); serializing a credential
    /// to a JSON snapshot would silently leak it through `tracing` /
    /// `problem-details` paths that round-trip the `ApiConfig` value
    /// for diagnostics. Deserialising always leaves it `None`; the
    /// `from_env` path repopulates from the env var.
    #[serde(skip)]
    pub password: Option<SecretString>,
    /// Canonical `From` header (e.g. `"noreply@example.com"`). Applied
    /// to every outbound mail regardless of `EmailMessage` content, so
    /// a misconfigured caller cannot smuggle a different sender. The
    /// env-binding parser rejects values without an `@` at startup.
    pub from_address: String,
    /// TLS posture — see [`SmtpTlsMode`] for the per-variant semantics.
    pub tls: SmtpTlsMode,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiConfig;
    use crate::config::env::tests::env_guard;

    #[test]
    fn from_env_idempotency_defaults_to_memory() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Memory);
        assert_eq!(cfg.idempotency.ttl_secs, DEFAULT_TTL_SECS);
        assert_eq!(cfg.idempotency.max_entries, DEFAULT_MAX_ENTRIES);
        assert_eq!(
            cfg.idempotency.sweep_interval_secs,
            IdempotencyApiConfig::DEFAULT_SWEEP_INTERVAL_SECS
        );
    }

    #[test]
    fn from_env_idempotency_accepts_postgres_backend() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_IDEMPOTENCY_BACKEND", "postgres");
        env.set("API_IDEMPOTENCY_TTL_SECS", "3600");
        env.set("API_IDEMPOTENCY_SWEEP_INTERVAL_SECS", "120");

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Postgres);
        assert_eq!(cfg.idempotency.ttl_secs, 3600);
        assert_eq!(cfg.idempotency.sweep_interval_secs, 120);
    }

    #[test]
    fn from_env_idempotency_backend_is_case_insensitive() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_IDEMPOTENCY_BACKEND", "POSTGRES");

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Postgres);
    }

    #[test]
    fn from_env_idempotency_rejects_unknown_backend() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_IDEMPOTENCY_BACKEND", "redis");

        let err = ApiConfig::from_env().expect_err("unknown backend must error");
        match err {
            crate::config::ApiConfigError::ParseEnum { var, raw } => {
                assert_eq!(var, "IDEMPOTENCY_BACKEND");
                assert_eq!(raw, "redis");
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn from_env_idempotency_rejects_invalid_ttl() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_IDEMPOTENCY_TTL_SECS", "not-a-number");

        let err = ApiConfig::from_env().expect_err("non-numeric TTL must error");
        match err {
            crate::config::ApiConfigError::ParseInt { var, .. } => {
                assert_eq!(var, "IDEMPOTENCY_TTL_SECS");
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn for_test_idempotency_defaults_to_memory() {
        let cfg = ApiConfig::for_test();
        assert_eq!(cfg.idempotency.backend, IdempotencyBackend::Memory);
        assert!(cfg.idempotency.ttl_secs > 0);
    }

    #[test]
    fn from_env_idempotency_rejects_zero_ttl_secs() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_IDEMPOTENCY_TTL_SECS", "0");

        let err = ApiConfig::from_env().expect_err("ttl_secs=0 must error");
        match err {
            crate::config::ApiConfigError::ZeroValue { var } => {
                assert_eq!(var, "IDEMPOTENCY_TTL_SECS");
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn from_env_auth_backend_defaults_to_memory() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.auth.backend, AuthBackendKind::Memory);
    }

    #[test]
    fn from_env_auth_backend_accepts_postgres() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_AUTH_BACKEND", "postgres");

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.auth.backend, AuthBackendKind::Postgres);
    }

    #[test]
    fn from_env_auth_backend_is_case_insensitive() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_AUTH_BACKEND", "PostGres");

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.auth.backend, AuthBackendKind::Postgres);
    }

    #[test]
    fn from_env_auth_backend_rejects_unknown() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_AUTH_BACKEND", "ldap");

        let err = ApiConfig::from_env().expect_err("unknown backend must error");
        match err {
            crate::config::ApiConfigError::ParseEnum { var, raw } => {
                assert_eq!(var, "AUTH_BACKEND");
                assert_eq!(raw, "ldap");
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn for_test_auth_backend_defaults_to_memory() {
        let cfg = ApiConfig::for_test();
        assert_eq!(cfg.auth.backend, AuthBackendKind::Memory);
    }

    // ---- SMTP env binding (`API_SMTP_*`) ---------------------------------

    #[test]
    fn from_env_smtp_absent_keeps_none() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        let cfg = ApiConfig::from_env().expect("config must load");
        assert!(cfg.smtp.is_none(), "missing API_SMTP_HOST must yield None");
    }

    #[test]
    fn from_env_smtp_present_populates_full_config_with_defaults() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_SMTP_HOST", "smtp.example.com");
        env.set("API_SMTP_FROM", "noreply@example.com");
        // PORT, USERNAME, PASSWORD, TLS_MODE all unset — defaults apply.
        let cfg = ApiConfig::from_env().expect("config must load");
        let smtp = cfg.smtp.as_ref().expect("smtp must be populated");
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.port, 587, "default port is 587 (submission)");
        assert_eq!(smtp.from_address, "noreply@example.com");
        assert!(smtp.username.is_none());
        assert!(smtp.password.is_none());
        assert_eq!(
            smtp.tls,
            SmtpTlsMode::StartTls,
            "port 587 must default to STARTTLS"
        );
    }

    #[test]
    fn from_env_smtp_465_defaults_to_implicit_tls() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_SMTP_HOST", "smtp.example.com");
        env.set("API_SMTP_PORT", "465");
        env.set("API_SMTP_FROM", "noreply@example.com");
        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(
            cfg.smtp.as_ref().unwrap().tls,
            SmtpTlsMode::Implicit,
            "port 465 must default to implicit TLS"
        );
    }

    #[test]
    fn from_env_smtp_rejects_username_without_password() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_SMTP_HOST", "smtp.example.com");
        env.set("API_SMTP_FROM", "noreply@example.com");
        env.set("API_SMTP_USERNAME", "noreply@example.com");
        // PASSWORD intentionally omitted.
        let err = ApiConfig::from_env().expect_err("USERNAME without PASSWORD must fail closed");
        assert!(
            matches!(err, crate::config::ApiConfigError::SmtpAuthIncomplete),
            "wrong variant: {err:?}"
        );
    }

    #[test]
    fn from_env_smtp_rejects_password_without_username() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_SMTP_HOST", "smtp.example.com");
        env.set("API_SMTP_FROM", "noreply@example.com");
        env.set("API_SMTP_PASSWORD", "sekret");
        // USERNAME intentionally omitted.
        let err = ApiConfig::from_env().expect_err("PASSWORD without USERNAME must fail closed");
        assert!(
            matches!(err, crate::config::ApiConfigError::SmtpAuthIncomplete),
            "wrong variant: {err:?}"
        );
    }

    #[test]
    fn from_env_smtp_rejects_missing_from() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_SMTP_HOST", "smtp.example.com");
        // FROM intentionally omitted.
        let err = ApiConfig::from_env().expect_err("missing API_SMTP_FROM must error");
        assert!(
            matches!(err, crate::config::ApiConfigError::SmtpFromMissing),
            "wrong variant: {err:?}"
        );
    }

    #[test]
    fn from_env_smtp_rejects_from_without_at() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_SMTP_HOST", "smtp.example.com");
        env.set("API_SMTP_FROM", "not-an-email");
        let err = ApiConfig::from_env().expect_err("invalid API_SMTP_FROM must error");
        assert!(
            matches!(err, crate::config::ApiConfigError::SmtpFromInvalid),
            "wrong variant: {err:?}"
        );
    }

    #[test]
    fn from_env_smtp_rejects_unknown_tls_mode() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_SMTP_HOST", "smtp.example.com");
        env.set("API_SMTP_FROM", "noreply@example.com");
        env.set("API_SMTP_TLS_MODE", "weird");
        let err = ApiConfig::from_env().expect_err("unknown TLS mode must error");
        match err {
            crate::config::ApiConfigError::ParseEnum { var, raw } => {
                assert_eq!(var, "SMTP_TLS_MODE");
                assert_eq!(raw, "weird");
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn from_env_idempotency_rejects_zero_max_entries() {
        let mut env = env_guard();
        env.set("NEBULA_ENV", "production");
        env.set("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        env.set("API_IDEMPOTENCY_MAX_ENTRIES", "0");

        let err = ApiConfig::from_env().expect_err("max_entries=0 must error");
        match err {
            crate::config::ApiConfigError::ZeroValue { var } => {
                assert_eq!(var, "IDEMPOTENCY_MAX_ENTRIES");
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
