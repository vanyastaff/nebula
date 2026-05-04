//! Time-based One-Time Password (RFC 6238) helpers.
//!
//! Built on top of `totp-rs` so we get the otpauth URI rendering for free.
//! Standard parameters: SHA-1 / 6 digits / 30 s window / ±1 step skew.

use rand::Rng;
use totp_rs::{Algorithm, Secret, TOTP};

use super::error::AuthError;

/// Issuer string baked into the otpauth URI; shows up under the entry name
/// in the user's authenticator app.
pub const ISSUER: &str = "Nebula";

/// Standard 6-digit TOTP code length.
pub const DIGITS: usize = 6;

/// 30-second TOTP step.
pub const STEP_SECS: u64 = 30;

/// Mint a fresh TOTP secret and the otpauth URI for `account_name`.
///
/// Returns `(secret_base32, otpauth_uri)`.
pub fn mint_secret(account_name: &str) -> Result<(String, String), AuthError> {
    let mut bytes = [0u8; 20];
    rand::rng().fill_bytes(&mut bytes);
    let secret = Secret::Raw(bytes.to_vec());
    let secret_b32 = secret.to_encoded().to_string();

    let totp = TOTP::new(
        Algorithm::SHA1,
        DIGITS,
        1,
        STEP_SECS,
        secret
            .to_bytes()
            .map_err(|e| AuthError::Crypto(format!("totp secret: {e}")))?,
        Some(ISSUER.to_owned()),
        account_name.to_owned(),
    )
    .map_err(|e| AuthError::Crypto(format!("totp ctor: {e}")))?;

    Ok((secret_b32, totp.get_url()))
}

/// Verify `code` against `secret_base32` using the standard ±1-step skew.
pub fn verify_code(secret_base32: &str, code: &str) -> Result<bool, AuthError> {
    let secret_bytes = Secret::Encoded(secret_base32.to_owned())
        .to_bytes()
        .map_err(|_| AuthError::InvalidMfaCode)?;
    let totp = TOTP::new(
        Algorithm::SHA1,
        DIGITS,
        1,
        STEP_SECS,
        secret_bytes,
        Some(ISSUER.to_owned()),
        "verify".to_owned(),
    )
    .map_err(|e| AuthError::Crypto(format!("totp ctor: {e}")))?;
    totp.check_current(code)
        .map_err(|e| AuthError::Crypto(format!("totp check: {e}")))
}

/// Generate the current code for `secret_base32`. Used by tests and the
/// confirm-enrollment loop in [`crate::auth::InMemoryAuthBackend`].
#[doc(hidden)]
pub fn current_code(secret_base32: &str) -> Result<String, AuthError> {
    let secret_bytes = Secret::Encoded(secret_base32.to_owned())
        .to_bytes()
        .map_err(|_| AuthError::Crypto("invalid base32 secret".to_owned()))?;
    let totp = TOTP::new(
        Algorithm::SHA1,
        DIGITS,
        1,
        STEP_SECS,
        secret_bytes,
        Some(ISSUER.to_owned()),
        "current".to_owned(),
    )
    .map_err(|e| AuthError::Crypto(format!("totp ctor: {e}")))?;
    totp.generate_current()
        .map_err(|e| AuthError::Crypto(format!("totp generate: {e}")))
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
