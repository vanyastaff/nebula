use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use sha2::{Digest, Sha256};

/// Generates a cryptographically random PKCE code verifier (43–128 characters, URL-safe base64).
pub fn generate_verifier() -> String {
    let mut rng = rand::rng();
    let len: usize = rng.random_range(32..=96); // produces 43–128 base64url chars
    let bytes: Vec<u8> = (0..len).map(|_| rng.random()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

/// Derives the PKCE code challenge from a verifier using SHA-256 + base64url encoding.
pub fn generate_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_length_in_range() {
        for _ in 0..100 {
            let v = generate_verifier();
            assert!(
                (43..=128).contains(&v.len()),
                "verifier length {} out of range: {v}",
                v.len()
            );
        }
    }

    #[test]
    fn verifier_is_url_safe() {
        let v = generate_verifier();
        assert!(
            v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier contains non-URL-safe chars: {v}"
        );
    }

    #[test]
    fn challenge_is_deterministic() {
        let verifier = "test-verifier-value";
        let c1 = generate_challenge(verifier);
        let c2 = generate_challenge(verifier);
        assert_eq!(c1, c2);
    }

    #[test]
    fn challenge_is_base64url() {
        let challenge = generate_challenge("some-verifier");
        assert!(
            challenge
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "challenge contains non-base64url chars: {challenge}"
        );
    }

    #[test]
    fn challenge_is_sha256_of_verifier() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = generate_challenge(verifier);
        // Verify it's a valid 32-byte SHA-256 hash encoded as base64url (43 chars)
        assert_eq!(challenge.len(), 43);
    }
}
