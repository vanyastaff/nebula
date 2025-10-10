//! Cryptographic utilities for credential flows

use sha2::{Digest, Sha256};

/// Generate random state parameter for `OAuth2` (URL-safe base64)
#[must_use] 
pub fn generate_random_state() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let random_bytes: [u8; 32] = rng.r#gen();
    base64_url_encode(&random_bytes)
}

/// Generate PKCE code verifier (43-128 characters, URL-safe)
#[must_use] 
pub fn generate_pkce_verifier() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let random_bytes: [u8; 32] = rng.r#gen();
    base64_url_encode(&random_bytes)
}

/// Generate PKCE code challenge from verifier using S256 method
#[must_use] 
pub fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    base64_url_encode(&hash)
}

/// Encode bytes as URL-safe base64 (no padding)
fn base64_url_encode(input: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_state() {
        let state1 = generate_random_state();
        let state2 = generate_random_state();

        // Should be different
        assert_ne!(state1, state2);

        // Should be URL-safe (no + / =)
        assert!(!state1.contains('+'));
        assert!(!state1.contains('/'));
        assert!(!state1.contains('='));
    }

    #[test]
    fn test_pkce_flow() {
        let verifier = generate_pkce_verifier();
        let challenge = generate_code_challenge(&verifier);

        // Verifier should be 43 chars (32 bytes base64url)
        assert_eq!(verifier.len(), 43);

        // Challenge should also be 43 chars (SHA256 32 bytes)
        assert_eq!(challenge.len(), 43);

        // Same verifier should produce same challenge
        let challenge2 = generate_code_challenge(&verifier);
        assert_eq!(challenge, challenge2);

        // Different verifier should produce different challenge
        let verifier2 = generate_pkce_verifier();
        let challenge3 = generate_code_challenge(&verifier2);
        assert_ne!(challenge, challenge3);
    }

    #[test]
    fn test_pkce_rfc7636_example() {
        // Test vector from RFC 7636 Appendix B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        let challenge = generate_code_challenge(verifier);
        assert_eq!(challenge, expected_challenge);
    }
}
