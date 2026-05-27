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

    /// `API_SMTP_HOST` is set but the value is empty or whitespace-only.
    ///
    /// Treating `API_SMTP_HOST=""` as "unset" would silently fall
    /// back to `EchoSink` for an operator who intended to wire SMTP
    /// (typo or templating bug) — same fail-CLOSED rationale as
    /// [`Self::SmtpAuthIncomplete`]. The boundary between "unset" and
    /// "intentionally empty" lives in `std::env::var`: `Err` means
    /// genuinely unset, `Ok(empty)` means a deliberate (broken) value.
    #[error(
        "API_SMTP_HOST is set but empty; unset the variable to use EchoSink or provide a real host"
    )]
    SmtpHostEmpty,

    /// `API_SMTP_HOST` is set but `API_SMTP_USERNAME` / `API_SMTP_PASSWORD`
    /// are not both set or both unset.
    ///
    /// Username without a password (or password without a username) is
    /// always misconfiguration: an authenticated relay needs both, an
    /// unauthenticated relay needs neither, and silently dropping the
    /// half-credential would mask a typo until the SMTP server returns
    /// `AUTH required` at first send.
    #[error("API_SMTP_USERNAME and API_SMTP_PASSWORD must both be set or both unset")]
    SmtpAuthIncomplete,

    /// `API_SMTP_HOST` is set but `API_SMTP_FROM` is missing or empty.
    #[error("API_SMTP_FROM is required when API_SMTP_HOST is set")]
    SmtpFromMissing,

    /// `API_SMTP_FROM` is present but does not look like an email
    /// address (no `@`). Lettre rejects the value at send time anyway;
    /// we surface it here so the misconfiguration shows up at startup
    /// rather than as a silent post-send error path.
    #[error("API_SMTP_FROM does not contain '@' (got an unparseable mailbox)")]
    SmtpFromInvalid,
}
