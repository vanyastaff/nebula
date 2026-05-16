/// Typed errors returned by [`super::ApiConfig::from_env`] and friends.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiConfigError {
    /// `API_JWT_SECRET` unset in a non-dev `NEBULA_ENV`.
    #[error("API_JWT_SECRET is required in non-dev mode (NEBULA_ENV={0})")]
    MissingJwtSecret(String),

    /// `API_JWT_SECRET` is shorter than [`super::JwtSecret::MIN_BYTES`].
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
    /// Used by [`super::ApiConfig::from_env`] for `API_IDEMPOTENCY_BACKEND` and
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
