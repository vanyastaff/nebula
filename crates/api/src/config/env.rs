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
pub(crate) mod tests {
    /// Serializes env-var manipulation across tests in this module so
    /// parallel nextest execution does not clobber shared state. We do
    /// not pull in `temp-env`/`serial_test` just for this.
    pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Clears every env var `from_env` reads. Must be called inside
    /// the lock.
    pub(crate) fn clear_env() {
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
}
