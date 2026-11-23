//! Time-based One-Time Password (RFC 6238) helpers.
//!
//! Built on top of `totp-rs` so we get the otpauth URI rendering for free.
//! Standard parameters: SHA-1 / 6 digits / 30 s window / ±1 step skew.

use std::time::{SystemTime, UNIX_EPOCH};

use rand::Rng;
use totp_rs::{Algorithm, Builder, Secret, Totp};

use super::error::AuthError;

/// Issuer string baked into the otpauth URI; shows up under the entry name
/// in the user's authenticator app.
pub const ISSUER: &str = "Nebula";

/// Standard 6-digit TOTP code length.
pub const DIGITS: usize = 6;

/// 30-second TOTP step.
pub const STEP_SECS: u64 = 30;

/// Build the standard TOTP (SHA-1, [`DIGITS`], ±1 step, [`STEP_SECS`]).
fn totp(secret: Secret, account_name: &str) -> Result<Totp, AuthError> {
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(DIGITS as u8)
        .with_skew(1)
        .with_step_duration(STEP_SECS)
        .with_secret(secret)
        .with_issuer(Some(ISSUER))
        .with_account_name(account_name)
        .build()
        .map_err(|e| AuthError::Crypto(format!("totp ctor: {e}")))
}

/// Seconds since the Unix epoch, as an error rather than a panic when the
/// clock is set before it.
fn unix_now() -> Result<u64, AuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|e| AuthError::Crypto(format!("totp clock: {e}")))
}

/// Mint a fresh TOTP secret and the otpauth URI for `account_name`.
///
/// Returns `(secret_base32, otpauth_uri)`.
pub fn mint_secret(account_name: &str) -> Result<(String, String), AuthError> {
    let mut bytes = [0u8; 20];
    rand::rng().fill_bytes(&mut bytes);
    let secret = Secret::new_stack(bytes);
    let secret_b32 = secret.to_base32();
    let url = totp(secret, account_name)?
        .to_url()
        .map_err(|e| AuthError::Crypto(format!("totp url: {e}")))?;
    Ok((secret_b32, url))
}

/// Verify `code` against `secret_base32` using the standard ±1-step skew.
pub fn verify_code(secret_base32: &str, code: &str) -> Result<bool, AuthError> {
    let secret = Secret::try_from_base32(secret_base32).map_err(|_| AuthError::InvalidMfaCode)?;
    Ok(totp(secret, "verify")?.check(code, unix_now()?).is_some())
}

/// Generate the current code for `secret_base32`. Used by tests and the
/// confirm-enrollment loop in [`super::InMemoryAuthBackend`].
#[doc(hidden)]
pub fn current_code(secret_base32: &str) -> Result<String, AuthError> {
    let secret = Secret::try_from_base32(secret_base32)
        .map_err(|_| AuthError::Crypto("invalid base32 secret".to_owned()))?;
    Ok(totp(secret, "current")?.generate(unix_now()?).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_secret_returns_otpauth_uri_and_base32() {
        let (b32, uri) = mint_secret("alice@example.com").unwrap();
        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("issuer=Nebula"));
        assert!(uri.contains("alice%40example.com") || uri.contains("alice@example.com"));
        assert!(!b32.is_empty());
    }

    #[test]
    fn current_code_verifies_against_same_secret() {
        let (b32, _) = mint_secret("test@nebula.dev").unwrap();
        let code = current_code(&b32).unwrap();
        assert_eq!(code.len(), DIGITS);
        assert!(verify_code(&b32, &code).unwrap());
    }

    #[test]
    fn wrong_code_does_not_verify() {
        let (b32, _) = mint_secret("test@nebula.dev").unwrap();
        assert!(!verify_code(&b32, "000000").unwrap());
    }
}
