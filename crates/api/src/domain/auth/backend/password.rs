//! Argon2id password hashing.
//!
//! Parameters (m=19_456 KiB, t=2, p=1) match OWASP 2024 minimum
//! recommendations and produce a verify time of ~50 ms on a 2024 laptop —
//! within the operational SLO "login p99 < 200 ms" budget while keeping the
//! brute-force factor at ≥ 2^16.

use argon2::{
    Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::{SaltString, rand_core::OsRng},
};

use std::sync::{Arc, LazyLock};

use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use super::error::AuthError;

/// Bounds concurrent Argon2 work across the process.
///
/// Each hash or verify takes about 19 MiB and one core for tens of
/// milliseconds. Unbounded, a burst of logins or signups would fill the
/// blocking pool and grow memory with every request in flight; one permit per
/// core keeps the work at what the machine can actually run.
static ARGON2_PERMITS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| {
    Arc::new(Semaphore::new(
        std::thread::available_parallelism().map_or(2, std::num::NonZero::get),
    ))
});

/// Run Argon2 work on the blocking pool, under [`ARGON2_PERMITS`].
///
/// Calling [`hash_password`] or [`verify_password`] directly from an async
/// handler stalls a runtime worker for the whole computation, and every task
/// scheduled on that worker with it.
async fn off_runtime<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AuthError> + Send + 'static,
) -> Result<T, AuthError> {
    let permit = Arc::clone(&ARGON2_PERMITS)
        .acquire_owned()
        .await
        .map_err(|e| AuthError::Crypto(format!("argon2 permits: {e}")))?;
    // The permit moves into the job: a caller dropped at its deadline must
    // not free capacity while the hash it started is still running.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|e| AuthError::Crypto(format!("argon2 task: {e}")))?
}

/// [`hash_password`] on the blocking pool; see [`off_runtime`].
///
/// # Errors
/// Returns [`AuthError::Crypto`] if hashing fails or the task cannot run.
pub async fn hash_password_async(password: &str) -> Result<String, AuthError> {
    let password = Zeroizing::new(password.to_owned());
    off_runtime(move || hash_password(&password)).await
}

/// [`verify_password`] on the blocking pool; see [`off_runtime`].
///
/// # Errors
/// Returns [`AuthError::Crypto`] if the stored hash is malformed or the task
/// cannot run.
pub async fn verify_password_async(hash: &str, password: &str) -> Result<bool, AuthError> {
    let hash = hash.to_owned();
    let password = Zeroizing::new(password.to_owned());
    off_runtime(move || verify_password(&hash, &password)).await
}

/// Shared account admission policy for signup and offline operator enrollment.
/// Returns the normalized email and trimmed display name without retaining a password.
///
/// # Errors
/// Invalid account input is rejected before hashing or persistence.
pub fn validate_registration<'a>(
    email: &str,
    password: &str,
    display_name: &'a str,
) -> Result<(String, &'a str), AuthError> {
    let email = email.trim().to_lowercase();
    let display_name = display_name.trim();
    if email.is_empty()
        || !email.contains('@')
        || password.len() < 8
        || display_name.is_empty()
        || display_name.len() > 128
    {
        return Err(AuthError::InvalidCredentials);
    }
    Ok((email, display_name))
}

/// Hash `password` with Argon2id and return the encoded PHC string.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, Version::V0x13, params()?);
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Crypto(format!("argon2 hash: {e}")))
}

/// Verify `password` against an encoded PHC `hash`. Returns `Ok(true)` only
/// on a successful match; `Ok(false)` on a clean mismatch; `Err(...)` if the
/// stored hash is malformed.
pub fn verify_password(hash: &str, password: &str) -> Result<bool, AuthError> {
    let parsed =
        PasswordHash::new(hash).map_err(|e| AuthError::Crypto(format!("argon2 parse: {e}")))?;
    let argon2 = Argon2::default();
    Ok(argon2.verify_password(password.as_bytes(), &parsed).is_ok())
}

fn params() -> Result<Params, AuthError> {
    // m=19456 KiB ≈ 19 MiB; t=2 iterations; p=1 lane.
    Params::new(19_456, 2, 1, None).map_err(|e| AuthError::Crypto(format!("argon2 params: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_succeeds() {
        let h = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password(&h, "correct horse battery staple").unwrap());
    }

    #[test]
    fn verify_rejects_wrong_password() {
        let h = hash_password("hunter2").unwrap();
        assert!(!verify_password(&h, "hunter3").unwrap());
    }

    #[test]
    fn verify_rejects_malformed_hash() {
        let err = verify_password("not-a-phc-string", "any").unwrap_err();
        assert!(matches!(err, AuthError::Crypto(_)));
    }

    #[tokio::test]
    async fn async_variants_round_trip() {
        let h = hash_password_async("off the runtime").await.unwrap();
        assert!(verify_password_async(&h, "off the runtime").await.unwrap());
        assert!(!verify_password_async(&h, "off the runtimE").await.unwrap());
    }

    #[test]
    fn two_hashes_of_same_password_differ() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "salt must randomize the encoded hash");
        assert!(verify_password(&a, "same").unwrap());
        assert!(verify_password(&b, "same").unwrap());
    }
}
