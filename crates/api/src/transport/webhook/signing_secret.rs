//! API-private signing-secret generation for webhook registration.
//!
//! Registration owns generation because the one-time `whsec_` value is an
//! HTTP response concern. Decoding and credential resolution belong to the
//! deployment adapter that implements [`super::WebhookSecretResolver`].

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rand::Rng as _;

use super::SecretResolutionError;

/// Mint a fresh Standard Webhooks signing secret.
///
/// The returned string contains 32 bytes from the OS CSPRNG encoded with
/// standard Base64 and prefixed with `whsec_`.
#[must_use]
pub(crate) fn mint_whsec() -> String {
    let mut raw = [0_u8; 32];
    rand::rng().fill_bytes(&mut raw);
    format!("whsec_{}", BASE64_STANDARD.encode(raw))
}

/// Reject an empty signing secret returned by a resolver.
///
/// The resolver port is intentionally implementation-neutral, so every API
/// consumer applies this shared postcondition before constructing a webhook
/// action or invoking a provider factory.
pub(crate) fn validate_resolved_secret(secret: Vec<u8>) -> Result<Vec<u8>, SecretResolutionError> {
    if secret.is_empty() {
        return Err(SecretResolutionError::InvalidMaterial);
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_secret_has_expected_format_and_entropy_size() {
        let secret = mint_whsec();
        let encoded = secret
            .strip_prefix("whsec_")
            .expect("minted secret carries the private format prefix");
        let decoded = BASE64_STANDARD
            .decode(encoded)
            .expect("minted payload uses standard Base64");

        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn consecutive_mints_are_distinct() {
        assert_ne!(mint_whsec(), mint_whsec());
    }

    #[test]
    fn resolved_secret_postcondition_rejects_empty_material() {
        assert_eq!(
            validate_resolved_secret(Vec::new()),
            Err(SecretResolutionError::InvalidMaterial),
        );
        assert_eq!(validate_resolved_secret(vec![0x42]), Ok(vec![0x42]),);
    }
}
