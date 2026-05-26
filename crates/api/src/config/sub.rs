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
/// `PgAuthBackend` shipped in PR2 commit 3.
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
    /// Durable PostgreSQL-backed identity backend (lands in PR2 commit 3).
    /// Survives restart and is shared across replicas that point at the
    /// same database.
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
    /// [`crate::transport::webhook::bootstrap_webhook_activations`]
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

#[cfg(test)]
#[allow(
    unsafe_code,
    reason = "env::{set_var, remove_var} are unsafe under edition 2024"
)]
mod tests {
    use super::*;
    use crate::config::ApiConfig;
    use crate::config::env::tests::{clear_env, env_lock};

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
            crate::config::ApiConfigError::ParseEnum { var, raw } => {
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
            crate::config::ApiConfigError::ParseInt { var, .. } => {
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
            crate::config::ApiConfigError::ZeroValue { var } => {
                assert_eq!(var, "IDEMPOTENCY_TTL_SECS");
            },
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn from_env_auth_backend_defaults_to_memory() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
        }

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.auth.backend, AuthBackendKind::Memory);

        clear_env();
    }

    #[test]
    fn from_env_auth_backend_accepts_postgres() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_AUTH_BACKEND", "postgres");
        }

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.auth.backend, AuthBackendKind::Postgres);

        clear_env();
    }

    #[test]
    fn from_env_auth_backend_is_case_insensitive() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_AUTH_BACKEND", "PostGres");
        }

        let cfg = ApiConfig::from_env().expect("config must load");
        assert_eq!(cfg.auth.backend, AuthBackendKind::Postgres);

        clear_env();
    }

    #[test]
    fn from_env_auth_backend_rejects_unknown() {
        let _g = env_lock();
        clear_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("NEBULA_ENV", "production");
            std::env::set_var("API_JWT_SECRET", "this-is-a-32-byte-minimum-secret!!");
            std::env::set_var("API_AUTH_BACKEND", "ldap");
        }

        let err = ApiConfig::from_env().expect_err("unknown backend must error");
        match err {
            crate::config::ApiConfigError::ParseEnum { var, raw } => {
                assert_eq!(var, "AUTH_BACKEND");
                assert_eq!(raw, "ldap");
            },
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }

    #[test]
    fn for_test_auth_backend_defaults_to_memory() {
        let cfg = ApiConfig::for_test();
        assert_eq!(cfg.auth.backend, AuthBackendKind::Memory);
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
            crate::config::ApiConfigError::ZeroValue { var } => {
                assert_eq!(var, "IDEMPOTENCY_MAX_ENTRIES");
            },
            other => panic!("wrong variant: {other:?}"),
        }

        clear_env();
    }
}
