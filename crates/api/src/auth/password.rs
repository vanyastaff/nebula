//! Argon2id password hashing.
//!
//! Parameters (m=19_456 KiB, t=2, p=1) match OWASP 2024 minimum
//! recommendations and produce a verify time of ~50 ms on a 2024 laptop —
//! within the canon §11 "login p99 < 200 ms" budget while keeping the
//! brute-force factor at ≥ 2^16.

use argon2::{
    Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::{SaltString, rand_core::OsRng},
};

use super::error::AuthError;

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

    #[test]
    fn two_hashes_of_same_password_differ() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "salt must randomize the encoded hash");
        assert!(verify_password(&a, "same").unwrap());
        assert!(verify_password(&b, "same").unwrap());
    }
}
