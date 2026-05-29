use super::errors::ApiConfigError;

pub(super) fn parse_u64_env(suffix: &'static str, default: u64) -> Result<u64, ApiConfigError> {
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
pub(super) fn parse_positive_u64_env(
    suffix: &'static str,
    default: u64,
) -> Result<u64, ApiConfigError> {
    let value = parse_u64_env(suffix, default)?;
    if value == 0 {
        return Err(ApiConfigError::ZeroValue { var: suffix });
    }
    Ok(value)
}

pub(super) fn parse_usize_env(
    suffix: &'static str,
    default: usize,
) -> Result<usize, ApiConfigError> {
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
pub(super) fn parse_bool_env(suffix: &'static str, default: bool) -> Result<bool, ApiConfigError> {
    match nebula_env::flag(&format!("API_{suffix}")) {
        Ok(parsed) => Ok(parsed.unwrap_or(default)),
        Err(nebula_env::EnvError::Invalid { value, .. }) => Err(ApiConfigError::ParseEnum {
            var: suffix,
            raw: value,
        }),
        // Unset / non-Unicode: fall back to the default, matching the prior
        // `std::env::var(..).is_err()` behaviour.
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use nebula_env::testing::EnvGuard;

    /// Every env var the `from_env` readers consult. Cleared before each test
    /// so the ambient process environment cannot leak into assertions;
    /// [`EnvGuard`] records the prior values and restores them on drop.
    const KNOWN_VARS: &[&str] = &[
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
        "API_AUTH_BACKEND",
        "API_SMTP_HOST",
        "API_SMTP_PORT",
        "API_SMTP_USERNAME",
        "API_SMTP_PASSWORD",
        "API_SMTP_FROM",
        "API_SMTP_TLS_MODE",
    ];

    /// Acquire an [`EnvGuard`] with every [`KNOWN_VARS`] entry cleared
    /// (recorded for restoration on drop). The guard's process-global lock
    /// serializes env mutation across this module's tests, replacing the
    /// former hand-rolled `env_lock` + `clear_env` pair and their `unsafe`.
    pub(crate) fn env_guard() -> EnvGuard {
        let mut guard = EnvGuard::acquire();
        for key in KNOWN_VARS {
            guard.remove(key);
        }
        guard
    }
}
