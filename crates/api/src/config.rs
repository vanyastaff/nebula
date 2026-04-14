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
//! soft fallback. Tests must use [`ApiConfig::for_test`], gated
//! behind the `test-util` feature.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};

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
    /// - [`ApiConfigError::JwtSecretTooShort`] if the input is
    ///   shorter than [`Self::MIN_BYTES`] bytes.
    /// - [`ApiConfigError::JwtSecretIsDevPlaceholder`] if the input
    ///   matches the well-known development placeholder.
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
    #[error("API_JWT_SECRET is required in production mode (NEBULA_ENV={0})")]
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
    #[error(
        "API_JWT_SECRET matches the well-known development placeholder — refusing to start"
    )]
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
}

/// API Server Configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Host and port to bind (e.g. "0.0.0.0:8080")
    pub bind_address: SocketAddr,

    /// Request timeout
    pub request_timeout: Duration,

    /// Maximum request body size (bytes)
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
            .finish()
    }
}

impl ApiConfig {
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
    /// - malformed `API_BIND_ADDRESS`, `API_REQUEST_TIMEOUT`,
    ///   `API_MAX_BODY_SIZE`, `API_ENABLE_COMPRESSION`,
    ///   `API_ENABLE_TRACING`, or `API_RATE_LIMIT`
    pub fn from_env() -> Result<Self, ApiConfigError> {
        let env_mode =
            std::env::var("NEBULA_ENV").unwrap_or_else(|_| "development".to_string());
        let is_dev = matches!(env_mode.as_str(), "development" | "dev" | "local");

        let jwt_secret = match std::env::var("API_JWT_SECRET") {
            Ok(raw) => JwtSecret::new(raw)?,
            Err(_) if is_dev => {
                let secret = JwtSecret::generate_ephemeral();
                tracing::warn!(
                    nebula_env = %env_mode,
                    "API_JWT_SECRET unset; generated ephemeral secret for dev mode. \
                     Tokens will be invalidated on restart."
                );
                secret
            }
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
            .unwrap_or_else(|_| "2097152".to_string())
            .parse()
            .map_err(|source| ApiConfigError::ParseInt {
                var: "MAX_BODY_SIZE",
                source,
            })?;

        let cors_allowed_origins = std::env::var("API_CORS_ORIGINS")
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

        Ok(Self {
            bind_address,
            request_timeout,
            max_body_size,
            cors_allowed_origins,
            enable_compression,
            enable_tracing,
            jwt_secret,
            rate_limit_per_second,
            api_keys,
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
            bind_address: "127.0.0.1:0".parse().expect("static addr"),
            request_timeout: Duration::from_secs(30),
            max_body_size: 2 * 1024 * 1024,
            cors_allowed_origins: vec!["*".to_string()],
            enable_compression: false,
            enable_tracing: false,
            jwt_secret: JwtSecret::for_test_unchecked(
                "test-secret-for-integration-tests-0123456789",
            ),
            rate_limit_per_second: 100,
            api_keys: Vec::new(),
        }
    }
}

#[cfg(test)]
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
            }
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

        // Two successive loads must produce different ephemeral
        // secrets — this is the guarantee that ephemeral is random
        // per process rather than a derived constant.
        assert_ne!(cfg1.jwt_secret.as_bytes(), cfg2.jwt_secret.as_bytes());
        assert!(cfg1.jwt_secret.as_bytes().len() >= JwtSecret::MIN_BYTES);

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
        let secret =
            JwtSecret::new("this-is-a-32-byte-minimum-secret!!".to_string()).unwrap();
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
}
